use chrono::{DateTime, Utc};
use organism_knowledge::{FeedbackRecord, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_schema_v() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolStats {
    pub accepts: u32,
    pub rejects: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlockStats {
    pub accepts: u32,
    pub rejects: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Terseness {
    Concise,
    Standard,
    Verbose,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StyleProfile {
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
    pub generated_at: DateTime<Utc>,
    pub feedback_count: u32,
    pub accept_rate_overall: f32,
    pub by_tool: HashMap<String, ToolStats>,
    pub by_block_kind: HashMap<String, BlockStats>,
    pub preferred_terseness: Terseness,
    pub top_accepted_phrases: Vec<String>,
    pub top_rejected_phrases: Vec<String>,
}

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be", "been", "being", "to",
    "of", "in", "on", "at", "by", "for", "with", "from", "as", "it", "its", "this", "that",
    "these", "those", "if", "then", "else", "when", "do", "does", "did", "have", "has", "had",
    "not", "no", "yes", "you", "your", "we", "our", "they", "them", "he", "she", "error", "rust",
    "try", "expected", "found", "note", "help",
];

impl StyleProfile {
    pub fn empty() -> Self {
        Self {
            schema_v: default_schema_v(),
            generated_at: Utc::now(),
            feedback_count: 0,
            accept_rate_overall: 0.0,
            by_tool: HashMap::new(),
            by_block_kind: HashMap::new(),
            preferred_terseness: Terseness::Standard,
            top_accepted_phrases: Vec::new(),
            top_rejected_phrases: Vec::new(),
        }
    }
}

/// Build a StyleProfile from raw feedback + accepted-suggestion text + per-error metadata.
///
/// `accepted_text`: map suggestion_hash -> accepted suggestion text (loaded by caller from
///   immutable accepted_<suggestion_hash>.json store, NOT regenerable suggestion_<error_hash>.json).
/// `tool_for_hash`: map error_hash -> tool name (e.g. "rustc"). From ErrorRecord.tool.
/// `block_kind_for_suggestion`: map suggestion_hash -> "patch"|"shell"|"note". Caller pre-classifies.
pub fn build_profile(
    feedback: &[FeedbackRecord],
    accepted_text: &HashMap<String, String>,
    tool_for_hash: &HashMap<String, String>,
    block_kind_for_suggestion: &HashMap<String, String>,
) -> StyleProfile {
    if feedback.is_empty() {
        return StyleProfile::empty();
    }

    let total = feedback.len() as u32;
    let accepts_count = feedback
        .iter()
        .filter(|f| matches!(f.verdict, Verdict::Accepted))
        .count() as u32;
    let accept_rate_overall = accepts_count as f32 / total as f32;

    let mut by_tool: HashMap<String, ToolStats> = HashMap::new();
    let mut by_block_kind: HashMap<String, BlockStats> = HashMap::new();
    let mut phrase_counts: HashMap<String, u32> = HashMap::new();
    let mut accepted_line_counts: Vec<usize> = Vec::new();

    for f in feedback {
        // Tool lookup and counting
        if let Some(tool) = tool_for_hash.get(&f.error_hash) {
            let stats = by_tool.entry(tool.clone()).or_insert(ToolStats {
                accepts: 0,
                rejects: 0,
            });
            match f.verdict {
                Verdict::Accepted => stats.accepts += 1,
                Verdict::Rejected => stats.rejects += 1,
                Verdict::Ignored => {}
            }
        }

        // Block kind lookup and counting
        if let Some(kind) = block_kind_for_suggestion.get(&f.suggestion_hash) {
            let stats = by_block_kind.entry(kind.clone()).or_insert(BlockStats {
                accepts: 0,
                rejects: 0,
            });
            match f.verdict {
                Verdict::Accepted => stats.accepts += 1,
                Verdict::Rejected => stats.rejects += 1,
                Verdict::Ignored => {}
            }
        }

        // Phrase mining for accepted feedback
        if matches!(f.verdict, Verdict::Accepted) {
            if let Some(text) = accepted_text.get(&f.suggestion_hash) {
                // Count line count
                let line_count = text.lines().count().max(1);
                accepted_line_counts.push(line_count);

                // Tokenize and extract n-grams
                let lowercase_text = text.to_lowercase();
                let tokens: Vec<&str> = lowercase_text.split_whitespace().collect();

                // 2-grams
                for window in tokens.windows(2) {
                    if !is_fully_stopword(window) {
                        let phrase = format!("{} {}", window[0], window[1]);
                        *phrase_counts.entry(phrase).or_insert(0) += 1;
                    }
                }

                // 3-grams
                for window in tokens.windows(3) {
                    if !is_fully_stopword(window) {
                        let phrase = format!("{} {} {}", window[0], window[1], window[2]);
                        *phrase_counts.entry(phrase).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    // Top 10 phrases by frequency, tie-break by phrase string ASC
    let mut phrase_vec: Vec<(String, u32)> = phrase_counts.into_iter().collect();
    phrase_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top_accepted_phrases: Vec<String> =
        phrase_vec.into_iter().take(10).map(|(p, _)| p).collect();

    // Preferred terseness based on average line count of accepted suggestions
    let preferred_terseness = if accepted_line_counts.is_empty() {
        Terseness::Standard
    } else {
        let avg_lines =
            accepted_line_counts.iter().sum::<usize>() as f32 / accepted_line_counts.len() as f32;
        if avg_lines < 8.0 {
            Terseness::Concise
        } else if avg_lines <= 20.0 {
            Terseness::Standard
        } else {
            Terseness::Verbose
        }
    };

    StyleProfile {
        schema_v: 1,
        generated_at: Utc::now(),
        feedback_count: total,
        accept_rate_overall,
        by_tool,
        by_block_kind,
        preferred_terseness,
        top_accepted_phrases,
        top_rejected_phrases: Vec::new(), // TODO(M10): rejected text source — needs separate suggestion archive
    }
}

fn is_fully_stopword(tokens: &[&str]) -> bool {
    tokens.iter().all(|t| STOPWORDS.contains(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_roundtrips() {
        let p = StyleProfile::empty();
        let json = serde_json::to_string(&p).unwrap();
        let back: StyleProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn mixed_profile_roundtrips() {
        let mut by_tool = HashMap::new();
        by_tool.insert(
            "rustc".to_string(),
            ToolStats {
                accepts: 5,
                rejects: 2,
            },
        );
        let mut by_block_kind = HashMap::new();
        by_block_kind.insert(
            "patch".to_string(),
            BlockStats {
                accepts: 3,
                rejects: 0,
            },
        );
        let p = StyleProfile {
            schema_v: 1,
            generated_at: Utc::now(),
            feedback_count: 7,
            accept_rate_overall: 0.71,
            by_tool,
            by_block_kind,
            preferred_terseness: Terseness::Concise,
            top_accepted_phrases: vec!["cargo build".into(), "use std".into()],
            top_rejected_phrases: vec!["the the".into()],
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: StyleProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn terseness_serializes_snake_case() {
        let s = serde_json::to_string(&Terseness::Standard).unwrap();
        assert_eq!(s, "\"standard\"");
        let v = serde_json::to_string(&Terseness::Verbose).unwrap();
        assert_eq!(v, "\"verbose\"");
        let c = serde_json::to_string(&Terseness::Concise).unwrap();
        assert_eq!(c, "\"concise\"");
    }

    #[test]
    fn missing_schema_v_defaults_to_1() {
        let json = r#"{
            "generated_at": "2026-01-01T00:00:00Z",
            "feedback_count": 0,
            "accept_rate_overall": 0.0,
            "by_tool": {},
            "by_block_kind": {},
            "preferred_terseness": "standard",
            "top_accepted_phrases": [],
            "top_rejected_phrases": []
        }"#;
        let p: StyleProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.schema_v, 1);
    }

    fn fb(verdict: Verdict, error_hash: &str, sugg_hash: &str) -> FeedbackRecord {
        FeedbackRecord {
            error_hash: error_hash.to_string(),
            suggestion_hash: sugg_hash.to_string(),
            verdict,
            note: None,
            ts: Utc::now(),
            schema_v: 1,
        }
    }

    #[test]
    fn build_profile_empty_input() {
        let feedback = vec![];
        let profile = build_profile(&feedback, &HashMap::new(), &HashMap::new(), &HashMap::new());

        assert_eq!(profile.feedback_count, 0);
        assert_eq!(profile.accept_rate_overall, 0.0);
        assert!(profile.by_tool.is_empty());
        assert!(profile.by_block_kind.is_empty());
    }

    #[test]
    fn build_profile_block_kind_counts() {
        let feedback = (0..10)
            .map(|i| fb(Verdict::Accepted, &format!("err_{}", i), "h1"))
            .collect::<Vec<_>>();

        let mut block_kind = HashMap::new();
        block_kind.insert("h1".to_string(), "patch".to_string());

        let profile = build_profile(&feedback, &HashMap::new(), &HashMap::new(), &block_kind);

        assert_eq!(profile.by_block_kind["patch"].accepts, 10);
        assert_eq!(profile.by_block_kind["patch"].rejects, 0);
    }

    #[test]
    fn build_profile_tool_split() {
        let mut feedback = vec![];
        for i in 0..5 {
            feedback.push(fb(Verdict::Accepted, &format!("rustc_{}", i), "s1"));
        }
        for i in 0..5 {
            feedback.push(fb(Verdict::Rejected, &format!("npm_{}", i), "s2"));
        }

        let mut tool_map = HashMap::new();
        tool_map.insert("rustc_0".to_string(), "rustc".to_string());
        tool_map.insert("rustc_1".to_string(), "rustc".to_string());
        tool_map.insert("rustc_2".to_string(), "rustc".to_string());
        tool_map.insert("rustc_3".to_string(), "rustc".to_string());
        tool_map.insert("rustc_4".to_string(), "rustc".to_string());
        tool_map.insert("npm_0".to_string(), "npm".to_string());
        tool_map.insert("npm_1".to_string(), "npm".to_string());
        tool_map.insert("npm_2".to_string(), "npm".to_string());
        tool_map.insert("npm_3".to_string(), "npm".to_string());
        tool_map.insert("npm_4".to_string(), "npm".to_string());

        let profile = build_profile(&feedback, &HashMap::new(), &tool_map, &HashMap::new());

        assert_eq!(profile.by_tool["rustc"].accepts, 5);
        assert_eq!(profile.by_tool["rustc"].rejects, 0);
        assert_eq!(profile.by_tool["npm"].accepts, 0);
        assert_eq!(profile.by_tool["npm"].rejects, 5);
        assert_eq!(profile.accept_rate_overall, 0.5);
    }

    #[test]
    fn build_profile_terseness_verbose() {
        let feedback = vec![fb(Verdict::Accepted, "err", "h1")];

        let mut accepted_text = HashMap::new();
        let fifty_lines = "line\n".repeat(50);
        accepted_text.insert("h1".to_string(), fifty_lines);

        let profile = build_profile(&feedback, &accepted_text, &HashMap::new(), &HashMap::new());

        assert_eq!(profile.preferred_terseness, Terseness::Verbose);
    }

    #[test]
    fn build_profile_stopword_filtering() {
        let feedback = vec![fb(Verdict::Accepted, "err", "h1")];

        let mut accepted_text = HashMap::new();
        accepted_text.insert("h1".to_string(), "the the the cargo build".to_string());

        let profile = build_profile(&feedback, &accepted_text, &HashMap::new(), &HashMap::new());

        assert!(!profile
            .top_accepted_phrases
            .contains(&"the the".to_string()));
        assert!(profile
            .top_accepted_phrases
            .contains(&"cargo build".to_string()));
    }
}
