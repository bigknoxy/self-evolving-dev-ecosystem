use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::env;
use std::net::IpAddr;
use url::Url;

#[derive(Debug, Clone)]
pub struct OllamaClient {
    pub base_url: String,
    pub model: String,
    pub http: reqwest::Client,
}

/// Check if a hostname is a loopback address.
/// Recognizes:
/// - "localhost" string literal
/// - IPv4 loopback (127.0.0.0/8)
/// - IPv6 loopback (::1)
/// - IPv4-mapped IPv6 loopback (::ffff:127.x.x.x)
fn is_loopback_host(host: &str) -> bool {
    // Check localhost string first (fallback for URL hostname strings)
    if host == "localhost" {
        return true;
    }

    // Try to parse as IP address
    if let Ok(ip) = host.parse::<IpAddr>() {
        // Handle IPv4-mapped IPv6 addresses (e.g., ::ffff:127.0.0.1)
        if let IpAddr::V6(v6) = ip {
            if let Some(mapped_v4) = v6.to_ipv4_mapped() {
                return mapped_v4.is_loopback();
            }
        }

        // Handle standard loopback check (includes 127.0.0.0/8 for IPv4 and ::1 for IPv6)
        return ip.is_loopback();
    }

    false
}

/// Validate OLLAMA_BASE_URL for remote URLs with warning/strict mode
fn validate_base_url(base_url: &str) -> Result<()> {
    let url = Url::parse(base_url)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("OLLAMA_BASE_URL has no host: {}", base_url))?;

    if is_loopback_host(host) {
        // Local URL - no warning needed
        return Ok(());
    }

    // Remote URL - check strict and insecure flags
    let is_strict = env::var("OLLAMA_STRICT").map(|v| v == "1").unwrap_or(false);
    let is_insecure = env::var("OLLAMA_INSECURE")
        .map(|v| v == "1")
        .unwrap_or(false);

    if is_strict {
        // OLLAMA_STRICT=1 means refuse remote URLs
        return Err(anyhow::anyhow!(
            "OLLAMA_BASE_URL is remote ({}); set OLLAMA_STRICT=0 or use a local URL",
            host
        ));
    }

    // Default: warn loudly but proceed (unless OLLAMA_INSECURE=1 explicitly unsets the warning)
    if !is_insecure {
        tracing::warn!(
            "OLLAMA_BASE_URL is remote ({}); redaction is best-effort, do not enable on shared hosts",
            host
        );
    }

    Ok(())
}

impl OllamaClient {
    pub fn new() -> Result<Self> {
        let base_url =
            env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5-coder:7b".to_string());

        // Validate remote URL and warn/error as needed
        validate_base_url(&base_url)?;

        Ok(Self {
            base_url,
            model,
            http: reqwest::Client::new(),
        })
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

    #[test]
    fn test_is_loopback_localhost() {
        assert!(is_loopback_host("localhost"));
    }

    #[test]
    fn test_is_loopback_127_0_0_1() {
        assert!(is_loopback_host("127.0.0.1"));
    }

    #[test]
    fn test_is_loopback_ipv6() {
        assert!(is_loopback_host("::1"));
    }

    #[test]
    fn test_is_loopback_127_x_x_x() {
        assert!(is_loopback_host("127.1.2.3"));
        assert!(is_loopback_host("127.99.88.77"));
    }

    #[test]
    fn test_is_not_loopback_remote() {
        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host("192.168.1.1"));
        assert!(!is_loopback_host("10.0.0.1"));
    }

    #[test]
    fn test_ipv6_ipv4_mapped_loopback() {
        // IPv4-mapped IPv6 loopback addresses should be recognized
        assert!(is_loopback_host("::ffff:127.0.0.1"));
        assert!(is_loopback_host("::ffff:127.0.0.2"));
        assert!(is_loopback_host("::ffff:127.255.255.255"));
    }

    #[test]
    fn test_ipv6_ipv4_mapped_non_loopback() {
        // IPv4-mapped IPv6 non-loopback addresses should NOT be recognized
        assert!(!is_loopback_host("::ffff:192.168.1.1"));
        assert!(!is_loopback_host("::ffff:8.8.8.8"));
        assert!(!is_loopback_host("::ffff:10.0.0.1"));
    }

    #[test]
    fn test_is_loopback_malformed_ip() {
        // Malformed IPs should be rejected
        assert!(!is_loopback_host("127.0.0.0.1")); // extra octet
        assert!(!is_loopback_host("127.0.0")); // incomplete
        assert!(!is_loopback_host("not-an-ip"));
    }

    #[test]
    fn test_validate_local_url_no_warning() {
        let result = validate_base_url("http://localhost:11434");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_local_url_127_0_0_1() {
        let result = validate_base_url("http://127.0.0.1:11434");
        assert!(result.is_ok());
    }

    #[test]
    fn test_ipv6_loopback_recognized() {
        let result = validate_base_url("http://[::1]:11434");
        assert!(result.is_ok());
    }

    #[test]
    fn test_ipv6_ipv4_mapped_loopback_url() {
        let result = validate_base_url("http://[::ffff:127.0.0.1]:11434");
        assert!(result.is_ok());
    }

    // M8-03 PII Guard Tests - Per spec
    // Using a static mutex to serialize these tests and avoid env var conflicts
    static M8_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_m8_local_url_ok_silent() {
        // M8-03 Spec: local URL OK silent
        let _lock = M8_TEST_LOCK.lock().unwrap();
        let result = validate_base_url("http://localhost:11434");
        assert!(result.is_ok());
    }

    #[test]
    fn test_m8_ipv6_loopback_local() {
        // M8-03 Spec: IPv6 loopback ::1 recognized as local
        let _lock = M8_TEST_LOCK.lock().unwrap();
        let result = validate_base_url("http://[::1]:11434");
        assert!(result.is_ok());
    }

    #[test]
    fn test_m8_remote_url_default_warns_proceeds() {
        // M8-03 Spec: remote URL warns but proceeds (default, no OLLAMA_STRICT)
        let _lock = M8_TEST_LOCK.lock().unwrap();

        let orig_strict = std::env::var("OLLAMA_STRICT").ok();
        let orig_insecure = std::env::var("OLLAMA_INSECURE").ok();

        std::env::remove_var("OLLAMA_STRICT");
        std::env::remove_var("OLLAMA_INSECURE");

        let result = validate_base_url("http://example.com:11434");

        // Restore
        if let Some(v) = orig_strict {
            std::env::set_var("OLLAMA_STRICT", v);
        } else {
            std::env::remove_var("OLLAMA_STRICT");
        }
        if let Some(v) = orig_insecure {
            std::env::set_var("OLLAMA_INSECURE", v);
        } else {
            std::env::remove_var("OLLAMA_INSECURE");
        }

        // Should succeed (warning logged via tracing)
        assert!(result.is_ok());
    }

    #[test]
    fn test_m8_strict_flag_refuses_remote() {
        // M8-03 Spec: OLLAMA_STRICT=1 + remote URL refuses (anyhow::bail)
        let _lock = M8_TEST_LOCK.lock().unwrap();

        let orig_strict = std::env::var("OLLAMA_STRICT").ok();
        let orig_insecure = std::env::var("OLLAMA_INSECURE").ok();

        std::env::set_var("OLLAMA_STRICT", "1");
        std::env::remove_var("OLLAMA_INSECURE");

        let result = validate_base_url("http://example.com:11434");

        // Restore
        if let Some(v) = orig_strict {
            std::env::set_var("OLLAMA_STRICT", v);
        } else {
            std::env::remove_var("OLLAMA_STRICT");
        }
        if let Some(v) = orig_insecure {
            std::env::set_var("OLLAMA_INSECURE", v);
        } else {
            std::env::remove_var("OLLAMA_INSECURE");
        }

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("remote"));
    }
}
