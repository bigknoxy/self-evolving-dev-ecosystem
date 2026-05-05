use super::redact;
use anyhow::Result;
use organism_knowledge::KnowledgeStore;
pub use organism_ollama::LlmClient;

/// Suggest remediation steps for a given error based on LLM.
///
/// Builds a prompt from the stored error record and sends it to the LLM client,
/// which can be either a real Ollama client or a mock for testing.
/// Sensitive data (paths, credentials) are redacted from the prompt.
pub async fn suggest_for_error<C: LlmClient>(
    client: &C,
    store: &mut KnowledgeStore,
    error_key: &str,
) -> Result<String> {
    let error = store
        .get_error(error_key)?
        .ok_or_else(|| anyhow::anyhow!("Error record not found: {}", error_key))?;

    let exit_code = "unknown".to_string();

    // Redact sensitive data from command, stderr snippet, and tool before including in prompt
    let redacted_command = redact(&error.last_command);
    let redacted_snippet = redact(&error.raw_excerpt);
    let redacted_tool = redact(&error.tool);

    let prompt = format!(
        "You are an expert {} dev. Last failure:\nCommand: {}\nExit: {}\nSnippet: {}\nOccurred {}x.\nGive 1–3 concrete next steps. Terse. Code blocks where useful.",
        redacted_tool,
        redacted_command,
        exit_code,
        redacted_snippet,
        error.occurrences
    );

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
        };

        store.put_error(&error).unwrap();

        let mock = MockLlmClient {
            response: "Try implementing the trait.".to_string(),
            received_prompt: std::sync::Arc::new(std::sync::Mutex::new(String::new())),
        };

        let suggestion = suggest_for_error(&mock, &mut store, "test_hash")
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

        let result = suggest_for_error(&mock, &mut store, "nonexistent").await;
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
        };

        store.put_error(&error).unwrap();

        struct FailingLlm;

        #[async_trait::async_trait]
        impl LlmClient for FailingLlm {
            async fn generate(&self, _prompt: &str) -> Result<String> {
                Err(anyhow::anyhow!("LLM unavailable"))
            }
        }

        let result = suggest_for_error(&FailingLlm, &mut store, "hash2").await;
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
        };

        store.put_error(&error).unwrap();

        let prompt_capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let mock = MockLlmClient {
            response: "Fix the error".to_string(),
            received_prompt: prompt_capture.clone(),
        };

        let _ = suggest_for_error(&mock, &mut store, "hash3").await.unwrap();

        let captured_prompt = prompt_capture.lock().unwrap();
        // Verify that the prompt contains redacted content
        assert!(captured_prompt.contains("$HOME"));
        assert!(!captured_prompt.contains("/Users/alice"));
        assert!(captured_prompt.contains("<KEY>=<REDACTED>"));
        assert!(!captured_prompt.contains("secret123"));
    }
}
