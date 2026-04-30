use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone)]
pub struct OllamaClient {
    pub base_url: String,
    pub model: String,
    pub http: reqwest::Client,
}

impl Default for OllamaClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OllamaClient {
    pub fn new() -> Self {
        let base_url =
            env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5-coder:7b".to_string());
        Self {
            base_url,
            model,
            http: reqwest::Client::new(),
        }
    }

    pub async fn generate(&self, prompt: &str) -> Result<String> {
        let url = format!("{}/api/generate", self.base_url);
        let payload = serde_json::json!({
            "model": &self.model,
            "prompt": prompt,
            "stream": false
        });

        let res = self.http.post(&url).json(&payload).send().await?;
        if !res.status().is_success() {
            return Err(anyhow::anyhow!(
                "Request failed with status: {}",
                res.status()
            ));
        }

        let body: OllamaResponse = res.json().await?;
        Ok(body.response)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaResponse {
    pub response: String,
}

/// LlmClient trait for testable, mockable LLM interactions.
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn generate(&self, prompt: &str) -> Result<String>;
}

#[async_trait]
impl LlmClient for OllamaClient {
    async fn generate(&self, prompt: &str) -> Result<String> {
        OllamaClient::generate(self, prompt).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_generate_success() {
        let mock_server = MockServer::start().await;
        let base_url = mock_server.uri();

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"response": "Hello!"}"#))
            .mount(&mock_server)
            .await;

        let client = OllamaClient {
            base_url,
            model: "test".to_string(),
            http: reqwest::Client::new(),
        };

        let result = client.generate("hi").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Hello!");
    }

    #[tokio::test]
    async fn test_generate_500_error() {
        let mock_server = MockServer::start().await;
        let base_url = mock_server.uri();

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client = OllamaClient {
            base_url,
            model: "test".to_string(),
            http: reqwest::Client::new(),
        };

        let result = client.generate("hi").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_generate_malformed_json() {
        let mock_server = MockServer::start().await;
        let base_url = mock_server.uri();

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock_server)
            .await;

        let client = OllamaClient {
            base_url,
            model: "test".to_string(),
            http: reqwest::Client::new(),
        };

        let result = client.generate("hi").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_generate_timeout() {
        let mock_server = MockServer::start().await;
        let base_url = mock_server.uri();

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&mock_server)
            .await;

        let client = OllamaClient {
            base_url,
            model: "test".to_string(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_millis(500))
                .build()
                .unwrap(),
        };

        let result = client.generate("hi").await;
        assert!(result.is_err());
    }
}
