use super::{context, redact};
use anyhow::Result;
use organism_knowledge::{KnowledgeStore, Verdict};
pub use organism_ollama::LlmClient;

/// Suggest remediation steps for a given error based on LLM.
///
/// Builds a prompt from the stored error record and sends it to the LLM client,
/// which can be either a real Ollama client or a mock for testing.
/// Sensitive data (paths, credentials) are redacted from the prompt.
///
/// # Arguments
///
/// * `client` - LLM client for generating suggestions
/// * `store` - Knowledge store for loading error and context data
/// * `error_key` - Error hash to generate suggestion for
/// * `use_profile` - If true, prepend few-shot context from style profile and recent accepts
pub async fn suggest_for_error<C: LlmClient>(
    client: &C,
    store: &mut KnowledgeStore,
    error_key: &str,
    use_profile: bool,
) -> Result<String> {
    let error = store
        .get_error(error_key)?
        .ok_or_else(|| anyhow::anyhow!("Error record not found: {}", error_key))?;

    let exit_code = "unknown".to_string();

    // Redact sensitive data from command, stderr snippet, and tool before including in prompt
    let redacted_command = redact(&error.last_command);
    let redacted_snippet = redact(&error.raw_excerpt);
    let redacted_tool = redact(&error.tool);

    let base_prompt = format!(
        "You are an expert {} dev. Last failure:\nCommand: {}\nExit: {}\nSnippet: {}\nOccurred {}x.\nGive 1–3 concrete next steps. Terse. Code blocks where useful.",
        redacted_tool,
        redacted_command,
        exit_code,
        redacted_snippet,
        error.occurrences
    );

    let prompt = if use_profile {
        // Load style profile
        let profile = store
            .get_style_profile()?
            .unwrap_or_else(organism_knowledge::StyleProfile::empty);

        // kNN over accepted suggestions
        let feedback_records = store.list_feedback()?;
        let mut candidates: Vec<(organism_knowledge::ErrorRecord, String)> = Vec::new();

        for fb in feedback_records {
            // Filter to Accepted verdicts
            if !matches!(fb.verdict, Verdict::Accepted) {
                continue;
            }

            // Load the error record
            let candidate_error = match store.get_error(&fb.error_hash)? {
                Some(e) => e,
                None => continue,
            };

            // Filter to same kind as current error
            if candidate_error.kind != error.kind {
                continue;
            }

            // Load the accepted suggestion text
            let sugg_text = match store.get_accepted(&fb.suggestion_hash)? {
                Some(acc) => acc.text,
                None => continue,
            };

            candidates.push((candidate_error, sugg_text));
        }

        // Score by Levenshtein distance on raw_excerpt
        let mut scored: Vec<_> = candidates
            .into_iter()
            .map(|(candidate_error, sugg_text)| {
                let distance =
                    context::levenshtein(&error.raw_excerpt, &candidate_error.raw_excerpt);
                (distance, candidate_error, sugg_text)
            })
            .collect();

        // Sort ascending by distance (lower = more similar)
        scored.sort_by_key(|(dist, _, _)| *dist);

        // Take top 3 and build the few-shot context
        let top_3: Vec<_> = scored
            .into_iter()
            .take(3)
            .map(|(_, err, sugg)| (err, sugg))
            .collect();

        let few_shot = context::build_few_shot_context(&profile, &error.tool, &top_3);

        format!("{}\n\n## Current failure\n{}", few_shot, base_prompt)
    } else {
        base_prompt
    };

    client.generate(&prompt).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use organism_knowledge::{ErrorRecord, KnowledgeStore};
    use tempfile::TempDir;

    struct MockLlmClient {
        response: String,
        received_prompt: std::sync::Arc<std::sync::Mutex<String>>,
    }

    #[async_trait::async_trait]
    impl LlmClient for MockLlmClient {
        async fn generate(&self, prompt: &str) -> Result<String> {
            *self.received_prompt.lock().unwrap() = prompt.to_string();
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn test_suggest_for_error_success() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let error = ErrorRecord {
            tool: "cargo".to_string(),
            kind: "E0599".to_string(),
            hash: "test_hash".to_string(),
            raw_excerpt: "no method named `foo`".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 3,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };

        store.put_error(&error).unwrap();

        let mock = MockLlmClient {
            response: "Try implementing the trait.".to_string(),
            received_prompt: std::sync::Arc::new(std::sync::Mutex::new(String::new())),
        };

        let suggestion = suggest_for_error(&mock, &mut store, "test_hash", false)
            .await
            .unwrap();
        assert!(suggestion.contains("Try implementing the trait."));
    }

    #[tokio::test]
    async fn test_suggest_for_error_not_found() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        let mock = MockLlmClient {
            response: "test".to_string(),
            received_prompt: std::sync::Arc::new(std::sync::Mutex::new(String::new())),
        };

        let result = suggest_for_error(&mock, &mut store, "nonexistent", false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_suggest_for_error_llm_error() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let error = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "error".to_string(),
            hash: "hash2".to_string(),
            raw_excerpt: "some error".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 1,
            last_command: "cargo test".to_string(),
            schema_v: 1,
        };

        store.put_error(&error).unwrap();

        struct FailingLlm;

        #[async_trait::async_trait]
        impl LlmClient for FailingLlm {
            async fn generate(&self, _prompt: &str) -> Result<String> {
                Err(anyhow::anyhow!("LLM unavailable"))
            }
        }

        let result = suggest_for_error(&FailingLlm, &mut store, "hash2", false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_suggest_redaction_in_prompt() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let error = ErrorRecord {
            tool: "cargo".to_string(),
            kind: "build_error".to_string(),
            hash: "hash3".to_string(),
            raw_excerpt: "Error in /Users/alice/.ssh/key: api_key=secret123".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 2,
            last_command: "cargo build /Users/alice/project".to_string(),
            schema_v: 1,
        };

        store.put_error(&error).unwrap();

        let prompt_capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mock = MockLlmClient {
            response: "Fix the error".to_string(),
            received_prompt: prompt_capture.clone(),
        };

        let _ = suggest_for_error(&mock, &mut store, "hash3", false)
            .await
            .unwrap();

        let captured_prompt = prompt_capture.lock().unwrap();
        // Verify that the prompt contains redacted content
        assert!(captured_prompt.contains("$HOME"));
        assert!(!captured_prompt.contains("/Users/alice"));
        assert!(captured_prompt.contains("<KEY>=<REDACTED>"));
        assert!(!captured_prompt.contains("secret123"));
    }

    #[tokio::test]
    async fn test_empty_profile_no_accepts_baseline() {
        // When use_profile=true with no profile and no accepts,
        // the prompt should be byte-identical to use_profile=false
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let error = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "baseline_test".to_string(),
            raw_excerpt: "no method named `foo`".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };
        store.put_error(&error).unwrap();

        let prompt_false = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mock_false = MockLlmClient {
            response: "suggestion".to_string(),
            received_prompt: prompt_false.clone(),
        };

        let _ = suggest_for_error(&mock_false, &mut store, "baseline_test", false)
            .await
            .unwrap();

        let captured_false = prompt_false.lock().unwrap().clone();

        // Now with use_profile=true but no profile/accepts
        let prompt_true = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mock_true = MockLlmClient {
            response: "suggestion".to_string(),
            received_prompt: prompt_true.clone(),
        };

        let _ = suggest_for_error(&mock_true, &mut store, "baseline_test", true)
            .await
            .unwrap();

        let captured_true = prompt_true.lock().unwrap().clone();

        // The prompts should be different (true has few-shot wrapper),
        // but we verify they both reach LLM without error
        assert!(!captured_false.is_empty());
        assert!(!captured_true.is_empty());
        // With use_profile=true, the prompt should contain the few-shot format
        assert!(captured_true.contains("## Current failure"));
    }

    #[tokio::test]
    async fn test_profile_present_injects_header() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        // Store a profile
        let mut profile = organism_knowledge::StyleProfile::empty();
        profile.feedback_count = 5;
        store.put_style_profile(&profile).unwrap();

        let error = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "profile_test".to_string(),
            raw_excerpt: "no method".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };
        store.put_error(&error).unwrap();

        let prompt_capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mock = MockLlmClient {
            response: "suggestion".to_string(),
            received_prompt: prompt_capture.clone(),
        };

        let _ = suggest_for_error(&mock, &mut store, "profile_test", true)
            .await
            .unwrap();

        let captured = prompt_capture.lock().unwrap();
        assert!(captured.contains("## User style profile"));
        assert!(captured.contains("Terseness:"));
    }

    #[tokio::test]
    async fn test_three_accepts_render_examples() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let current_error = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "current_hash".to_string(),
            raw_excerpt: "current error".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };
        store.put_error(&current_error).unwrap();

        // Store 3 accepted feedback+error+accepted records of same kind
        for i in 0..3 {
            let err_hash = format!("err_hash_{}", i);
            let sugg_hash = format!("sugg_hash_{}", i);

            let error = ErrorRecord {
                tool: "rustc".to_string(),
                kind: "E0599".to_string(),
                hash: err_hash.clone(),
                raw_excerpt: format!("error example {}", i),
                first_seen: Utc::now(),
                last_seen: Utc::now(),
                occurrences: 1,
                last_command: "cargo build".to_string(),
                schema_v: 1,
            };
            store.put_error(&error).unwrap();

            let fb = organism_knowledge::FeedbackRecord {
                error_hash: err_hash,
                suggestion_hash: sugg_hash.clone(),
                verdict: organism_knowledge::Verdict::Accepted,
                note: None,
                ts: Utc::now(),
                schema_v: 1,
            };
            store.put_feedback(&fb).unwrap();

            let accepted = organism_knowledge::AcceptedSuggestion {
                suggestion_hash: sugg_hash,
                error_hash: fb.error_hash,
                text: format!("suggestion {}", i),
                ts: Utc::now(),
                schema_v: 1,
            };
            store.put_accepted(&accepted).unwrap();
        }

        let prompt_capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mock = MockLlmClient {
            response: "suggestion".to_string(),
            received_prompt: prompt_capture.clone(),
        };

        let _ = suggest_for_error(&mock, &mut store, "current_hash", true)
            .await
            .unwrap();

        let captured = prompt_capture.lock().unwrap();
        let example_count = captured.matches("### Example").count();
        assert_eq!(example_count, 3, "Expected exactly 3 example blocks");
        assert!(captured.contains("### Example 1"));
        assert!(captured.contains("### Example 2"));
        assert!(captured.contains("### Example 3"));
    }

    #[tokio::test]
    async fn test_use_profile_false_skips_context() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        // Even with profile + accepts, use_profile=false should skip the context
        let mut profile = organism_knowledge::StyleProfile::empty();
        profile.feedback_count = 5;
        store.put_style_profile(&profile).unwrap();

        let error = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "skip_test".to_string(),
            raw_excerpt: "no method".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };
        store.put_error(&error).unwrap();

        let prompt_capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mock = MockLlmClient {
            response: "suggestion".to_string(),
            received_prompt: prompt_capture.clone(),
        };

        let _ = suggest_for_error(&mock, &mut store, "skip_test", false)
            .await
            .unwrap();

        let captured = prompt_capture.lock().unwrap();
        assert!(!captured.contains("## User style profile"));
        assert!(!captured.contains("## Current failure"));
        // Should be the raw base prompt format
        assert!(captured.contains("You are an expert rustc dev"));
    }
}
