use chrono::Utc;
use organism_knowledge::{BlockStats, FeedbackRecord, StyleProfile, Terseness, ToolStats, Verdict};
use std::collections::HashMap;

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "is", "are", "was", "were", "be", "been", "being", "to",
    "of", "in", "on", "at", "by", "for", "with", "from", "as", "it", "its", "this", "that",
    "these", "those", "if", "then", "else", "when", "do", "does", "did", "have", "has", "had",
    "not", "no", "yes", "you", "your", "we", "our", "they", "them", "he", "she", "error", "rust",
    "try", "expected", "found", "note", "help",
];

/// Classify accepted suggestion text into block kind: "patch", "shell", or "note".
/// Rules:
/// - "patch": contains triple-backtick fence with diff/patch language, or starts with "--- "
/// - "shell": contains fenced shell or bash block, or starts with "$ "
/// - "note": everything else
pub fn classify_block_kind(text: &str) -> &'static str {
    // Check for triple-backtick blocks
    if let Some(idx) = text.find("```") {
        let after = &text[idx + 3..];
        // Extract language identifier (first word after ```)
        let lang_end = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let lang = after[..lang_end].to_lowercase();

        if lang == "diff" || lang == "patch" {
            return "patch";
        }
        if lang == "shell" || lang == "bash" || lang == "sh" {
            return "shell";
        }
    }

    // Check for markdown-style shell indicator (starts with $ )
    if text.trim_start().starts_with("$ ") {
        return "shell";
    }

    // Check for diff/patch indicator (starts with --- )
    if text.trim_start().starts_with("--- ") {
        return "patch";
    }

    // Default to note
    "note"
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

    // Applied counts as 2 in both numerator and denominator so accept_rate stays in [0,1].
    // A single Applied is worth twice as much as Accepted (stronger positive signal).
    let (accept_weight, total_weight) = feedback.iter().fold((0u32, 0u32), |(acc, tot), f| {
        let (a, t) = match f.verdict {
            Verdict::Applied => (2, 2),
            Verdict::Accepted => (1, 1),
            Verdict::Rejected | Verdict::Ignored => (0, 1),
        };
        (acc + a, tot + t)
    });
    let accept_rate_overall = if total_weight == 0 {
        0.0f32
    } else {
        accept_weight as f32 / total_weight as f32
    };

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
                Verdict::Accepted | Verdict::Applied => stats.accepts += 1,
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
                Verdict::Accepted | Verdict::Applied => stats.accepts += 1,
                Verdict::Rejected => stats.rejects += 1,
                Verdict::Ignored => {}
            }
        }

        // Phrase mining for accepted + applied feedback
        if matches!(f.verdict, Verdict::Accepted | Verdict::Applied) {
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
        feedback_count: feedback.len() as u32,
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

    #[test]
    fn classify_block_kind_patch_fence() {
        assert_eq!(classify_block_kind("```patch\n- old\n+ new\n```"), "patch");
        assert_eq!(classify_block_kind("```diff\nsome diff here"), "patch");
    }

    #[test]
    fn classify_block_kind_shell_fence() {
        assert_eq!(classify_block_kind("```shell\necho hello"), "shell");
        assert_eq!(classify_block_kind("```bash\nls -la"), "shell");
        assert_eq!(classify_block_kind("```sh\ncd /tmp"), "shell");
    }

    #[test]
    fn classify_block_kind_note_default() {
        assert_eq!(classify_block_kind("just a note"), "note");
        assert_eq!(classify_block_kind("```\ngeneric fence"), "note");
        assert_eq!(classify_block_kind("some text here"), "note");
    }

    #[test]
    fn classify_block_kind_dollar_prefix() {
        assert_eq!(classify_block_kind("$ cargo build"), "shell");
        assert_eq!(classify_block_kind("   $ cargo test"), "shell");
    }

    #[test]
    fn classify_block_kind_diff_prefix() {
        assert_eq!(classify_block_kind("--- a/file.rs"), "patch");
        assert_eq!(classify_block_kind("   --- a/file.rs"), "patch");
    }

    #[test]
    fn build_profile_applied_weight_stays_in_bounds() {
        // 5 Applied + 5 Accepted: accept_weight=10+5=15, total_weight=10+5=15 → rate=1.0
        let mut feedback: Vec<FeedbackRecord> = (0..5)
            .map(|i| fb(Verdict::Applied, &format!("a{}", i), "s1"))
            .collect();
        feedback.extend((0..5).map(|i| fb(Verdict::Accepted, &format!("b{}", i), "s2")));

        let profile = build_profile(&feedback, &HashMap::new(), &HashMap::new(), &HashMap::new());
        assert!((profile.accept_rate_overall - 1.0).abs() < 1e-6, "rate={}", profile.accept_rate_overall);
        assert!(profile.accept_rate_overall <= 1.0, "rate must not exceed 1.0");
    }

    #[test]
    fn build_profile_applied_outweighs_rejected() {
        // 1 Applied (weight=2) vs 1 Rejected (weight=1): rate = 2/3 ≈ 0.667
        let feedback = vec![
            fb(Verdict::Applied, "a1", "s1"),
            fb(Verdict::Rejected, "a2", "s2"),
        ];
        let profile = build_profile(&feedback, &HashMap::new(), &HashMap::new(), &HashMap::new());
        let expected = 2.0f32 / 3.0;
        assert!((profile.accept_rate_overall - expected).abs() < 1e-5, "rate={}", profile.accept_rate_overall);
    }
}
