//! Integration test for M8 PII redaction in suggestion chain.
//! Seeds an ErrorRecord with PII (paths and secrets), runs the suggest pipeline,
//! and verifies that the prompt POSTed to the mock LLM contains redacted values.

use anyhow::Result;
use chrono::Utc;
use organism_cortex::{redact, suggest_for_error};
use organism_knowledge::{ErrorRecord, KnowledgeStore};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct M8TestContext {
    _tmp: TempDir,
    store: KnowledgeStore,
    mock_server: MockServer,
}

impl M8TestContext {
    async fn new() -> Result<Self> {
        let tmp = TempDir::new()?;
        let store = KnowledgeStore::open(tmp.path())?;
        let mock_server = MockServer::start().await;

        Ok(M8TestContext {
            _tmp: tmp,
            store,
            mock_server,
        })
    }
}

// Mock LLM client that captures the prompt sent to it
struct CapturePromptClient {
    base_url: String,
    captured_prompt: std::sync::Arc<std::sync::Mutex<String>>,
}

#[async_trait::async_trait]
impl organism_cortex::LlmClient for CapturePromptClient {
    async fn generate(&self, prompt: &str) -> Result<String> {
        *self.captured_prompt.lock().unwrap() = prompt.to_string();

        // Also simulate the HTTP POST to verify the body
        let client = reqwest::Client::new();
        let url = format!("{}/api/generate", self.base_url);
        let payload = serde_json::json!({
            "model": "qwen2.5-coder:7b",
            "prompt": prompt,
            "stream": false
        });

        match client.post(&url).json(&payload).send().await {
            Ok(res) => {
                if res.status().is_success() {
                    let body: serde_json::Value = res.json().await?;
                    Ok(body["response"].as_str().unwrap_or("").to_string())
                } else {
                    Err(anyhow::anyhow!("HTTP error: {}", res.status()))
                }
            }
            Err(e) => Err(anyhow::anyhow!("Request failed: {}", e)),
        }
    }
}

#[tokio::test]
async fn test_m8_redaction_chain_with_mock_llm() -> Result<()> {
    let mut ctx = M8TestContext::new().await?;
    let mock_server = &ctx.mock_server;

    // Mock the Ollama response
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"response": "Try checking the configuration file."}"#),
        )
        .mount(mock_server)
        .await;

    // Create an error record with PII
    let error = ErrorRecord {
        tool: "cargo".to_string(),
        kind: "compile_error".to_string(),
        hash: "pii_test_hash".to_string(),
        raw_excerpt: "Error accessing /Users/alice/secret/api_key=hunter2_super_secret".to_string(),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
        occurrences: 1,
        last_command: "cargo build /Users/alice/project".to_string(),
        schema_v: 1,
    };

    ctx.store.put_error(&error)?;

    // Create mock LLM client with prompt capture
    let captured_prompt = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let client = CapturePromptClient {
        base_url: mock_server.uri(),
        captured_prompt: captured_prompt.clone(),
    };

    // Call the suggest pipeline
    let suggestion = suggest_for_error(&client, &mut ctx.store, "pii_test_hash", false).await?;

    // Verify the response
    assert!(suggestion.contains("Try checking the configuration file."));

    // Verify the captured prompt contains redacted values
    let prompt = captured_prompt.lock().unwrap();
    println!("Captured prompt:\n{}", prompt);

    // Assert that PII is redacted
    assert!(
        prompt.contains("$HOME"),
        "Prompt should contain $HOME for redacted paths"
    );
    assert!(
        !prompt.contains("/Users/alice"),
        "Prompt should NOT contain /Users/alice"
    );
    assert!(
        prompt.contains("<KEY>=<REDACTED>"),
        "Prompt should contain <KEY>=<REDACTED> for redacted API keys"
    );
    assert!(
        !prompt.contains("hunter2_super_secret"),
        "Prompt should NOT contain the actual secret"
    );

    Ok(())
}

#[test]
fn test_m8_redact_function_directly() {
    // Direct unit test of the redact function
    let input = "Error in /Users/alice/.ssh/config with api_key=secret123 and alice@example.com";
    let redacted = redact(input);

    assert!(redacted.contains("$HOME"));
    assert!(!redacted.contains("/Users/alice"));
    assert!(redacted.contains("<KEY>=<REDACTED>"));
    assert!(!redacted.contains("secret123"));
    assert!(redacted.contains("<EMAIL>"));
    assert!(!redacted.contains("alice@example.com"));
}
