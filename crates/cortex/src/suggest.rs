use anyhow::Result;
use organism_knowledge::KnowledgeStore;
pub use organism_ollama::LlmClient;

/// Suggest remediation steps for a given error based on LLM.
///
/// Builds a prompt from the stored error record and sends it to the LLM client,
/// which can be either a real Ollama client or a mock for testing.
pub async fn suggest_for_error<C: LlmClient>(
    client: &C,
    store: &mut KnowledgeStore,
    error_key: &str,
) -> Result<String> {
    let error = store
        .get_error(error_key)?
        .ok_or_else(|| anyhow::anyhow!("Error record not found: {}", error_key))?;

    let exit_code = "unknown".to_string();
    let prompt = format!(
        "You are an expert {} dev. Last failure:\nCommand: {}\nExit: {}\nSnippet: {}\nOccurred {}x.\nGive 1–3 concrete next steps. Terse. Code blocks where useful.",
        error.tool,
        error.last_command,
        exit_code,
        error.raw_excerpt,
        error.occurrences
    );

    client.generate(&prompt).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use organism_knowledge::{ErrorRecord, KnowledgeStore};
    use tempfile::TempDir;

    struct MockLlmClient {
        response: String,
    }

    #[async_trait::async_trait]
    impl LlmClient for MockLlmClient {
        async fn generate(&self, _prompt: &str) -> Result<String> {
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
}
