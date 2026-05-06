//! Integration test for Ollama LLM suggestion flow.
//! Sets up a fake Ollama server with wiremock and verifies that
//! errors trigger suggestion generation.

use anyhow::Result;
use chrono::Utc;
use organism_cortex::suggest_for_error;
use organism_knowledge::{ErrorRecord, KnowledgeStore};
use organism_ollama::OllamaClient;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct TestContext {
    _tmp: TempDir,
    store: KnowledgeStore,
    #[allow(dead_code)]
    _mock_base_url: String,
}

impl TestContext {
    async fn new() -> Result<Self> {
        let tmp = TempDir::new()?;
        let store = KnowledgeStore::open(tmp.path())?;
        let mock_server = MockServer::start().await;
        let mock_base_url = mock_server.uri();

        Ok(TestContext {
            _tmp: tmp,
            store,
            _mock_base_url: mock_base_url,
        })
    }
}

#[tokio::test]
async fn test_suggest_for_error_with_mock_ollama() -> Result<()> {
    let mut ctx = TestContext::new().await?;

    // Set up wiremock to mock Ollama response
    let mock_server = MockServer::start().await;
    let base_url = mock_server.uri();

    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"response": "Try running `cargo fix --allow-dirty`."}"#),
        )
        .mount(&mock_server)
        .await;

    // Create an error record
    let error = ErrorRecord {
        tool: "cargo".to_string(),
        kind: "E0599".to_string(),
        hash: "test_hash_123".to_string(),
        raw_excerpt: "no method named `foo` found".to_string(),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
        occurrences: 2,
        last_command: "cargo build".to_string(),
        schema_v: 1,
    };

    ctx.store.put_error(&error)?;

    // Create OllamaClient with mocked base URL
    let client = OllamaClient {
        base_url,
        model: "test-model".to_string(),
        http: reqwest::Client::new(),
    };

    // Request suggestion
    let suggestion = suggest_for_error(&client, &mut ctx.store, "test_hash_123").await?;

    // Verify response contains expected text
    assert!(suggestion.contains("cargo fix"));
    Ok(())
}

#[tokio::test]
async fn test_suggest_handles_ollama_error() -> Result<()> {
    let mut ctx = TestContext::new().await?;

    let mock_server = MockServer::start().await;
    let base_url = mock_server.uri();

    // Mock an error response
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let error = ErrorRecord {
        tool: "rustc".to_string(),
        kind: "E0425".to_string(),
        hash: "err_hash_999".to_string(),
        raw_excerpt: "cannot find value".to_string(),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
        occurrences: 1,
        last_command: "cargo build".to_string(),
        schema_v: 1,
    };

    ctx.store.put_error(&error)?;

    let client = OllamaClient {
        base_url,
        model: "test-model".to_string(),
        http: reqwest::Client::new(),
    };

    let result = suggest_for_error(&client, &mut ctx.store, "err_hash_999").await;
    assert!(result.is_err(), "Should error on 500 response");
    Ok(())
}

#[tokio::test]
async fn test_suggest_error_not_found() -> Result<()> {
    let mut ctx = TestContext::new().await?;

    let mock_server = MockServer::start().await;
    let base_url = mock_server.uri();

    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"response": "test"}"#))
        .mount(&mock_server)
        .await;

    let client = OllamaClient {
        base_url,
        model: "test-model".to_string(),
        http: reqwest::Client::new(),
    };

    let result = suggest_for_error(&client, &mut ctx.store, "nonexistent").await;
    assert!(result.is_err(), "Should error when error record not found");
    assert!(result.unwrap_err().to_string().contains("not found"));
    Ok(())
}
