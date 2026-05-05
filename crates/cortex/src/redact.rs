use regex::Regex;
use std::sync::OnceLock;

/// Redact PII and sensitive data from text.
///
/// Applies the following redaction rules in order:
/// 1. Absolute home paths → $HOME/...
/// 2. API keys/tokens/secrets/passwords → <KEY>=<REDACTED>
/// 3. AWS access keys → <AWS_KEY_REDACTED>
/// 4. Bearer tokens → Bearer <REDACTED>
/// 5. Email addresses → <EMAIL>
pub fn redact(input: &str) -> String {
    let mut result = input.to_string();

    // Rule 1: Home paths - use literal string replacement ($ is not a regex group in closures)
    result = home_path_regex()
        .replace_all(&result, |_: &regex::Captures| "$HOME".to_string())
        .to_string();

    // Rule 2: Bearer tokens - process before generic API key regex to avoid double-redaction
    result = bearer_regex()
        .replace_all(&result, |_: &regex::Captures| {
            "Bearer <REDACTED>".to_string()
        })
        .to_string();

    // Rule 3: API keys, tokens, secrets, passwords
    // Skip replacement if it's "token: Bearer <REDACTED>" (already redacted)
    result = api_key_regex()
        .replace_all(&result, |caps: &regex::Captures| {
            let matched = &caps[0];
            if matched.to_lowercase().starts_with("token:") && matched.contains("Bearer") {
                matched.to_string()
            } else {
                "<KEY>=<REDACTED>".to_string()
            }
        })
        .to_string();

    // Rule 4: AWS access keys
    result = aws_key_regex()
        .replace_all(&result, |_: &regex::Captures| {
            "<AWS_KEY_REDACTED>".to_string()
        })
        .to_string();

    // Rule 5: Email addresses
    result = email_regex()
        .replace_all(&result, |_: &regex::Captures| "<EMAIL>".to_string())
        .to_string();

    result
}

/// Regex for absolute home paths: /Users/[username] or /home/[username]
fn home_path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?:/Users/[^/\s]+|/home/[^/\s]+)").expect("home path regex is valid")
    })
}

/// Regex for API keys, tokens, secrets, passwords
/// Matches: api_key=secret, API-KEY: hunter2, token:xyz, etc.
fn api_key_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)(api[_-]?key|token|secret|password)\s*[:=]\s*\S+")
            .expect("api key regex is valid")
    })
}

/// Regex for AWS access keys (format: AKIA followed by 16 alphanumeric chars)
fn aws_key_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"AKIA[0-9A-Z]{16}").expect("aws key regex is valid"))
}

/// Regex for Bearer tokens
fn bearer_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"Bearer\s+\S+").expect("bearer regex is valid"))
}

/// Regex for email addresses
fn email_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"[\w.+-]+@[\w-]+\.\w+").expect("email regex is valid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_home_path() {
        let input = "Error in /Users/alice/project/src/main.rs";
        let redacted = redact(input);
        assert!(redacted.contains("$HOME"));
        assert!(!redacted.contains("/Users/alice"));
    }

    #[test]
    fn test_redact_api_key() {
        let input = "Failed to authenticate: api_key=hunter2secret";
        let redacted = redact(input);
        assert!(redacted.contains("<KEY>=<REDACTED>"));
        assert!(!redacted.contains("hunter2secret"));
    }

    #[test]
    fn test_redact_api_key_with_dash() {
        let input = "Config error: API-KEY: super_secret_123";
        let redacted = redact(input);
        assert!(redacted.contains("<KEY>=<REDACTED>"));
        assert!(!redacted.contains("super_secret_123"));
    }

    #[test]
    fn test_redact_token() {
        let input = "Authorization failed: token=xyz789abc";
        let redacted = redact(input);
        assert!(redacted.contains("<KEY>=<REDACTED>"));
        assert!(!redacted.contains("xyz789abc"));
    }

    #[test]
    fn test_redact_aws_key() {
        let input = "AWS error accessing AKIAIOSFODNN7EXAMPLE";
        let redacted = redact(input);
        assert!(redacted.contains("<AWS_KEY_REDACTED>"));
        assert!(!redacted.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn test_redact_bearer() {
        let input = "Invalid token: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let redacted = redact(input);
        assert!(redacted.contains("Bearer <REDACTED>"));
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
    }

    #[test]
    fn test_redact_email() {
        let input = "Notification sent to alice@example.com for review";
        let redacted = redact(input);
        assert!(redacted.contains("<EMAIL>"));
        assert!(!redacted.contains("alice@example.com"));
    }

    #[test]
    fn test_redact_combined() {
        let input = "Error in /Users/bob/app, api_key=secret123, contact bob@example.com, AWS: AKIAIOSFODNN7EXAMPLE";
        let redacted = redact(input);
        assert!(redacted.contains("$HOME"));
        assert!(redacted.contains("<KEY>=<REDACTED>"));
        assert!(redacted.contains("<EMAIL>"));
        assert!(redacted.contains("<AWS_KEY_REDACTED>"));
        assert!(!redacted.contains("/Users/bob"));
        assert!(!redacted.contains("secret123"));
        assert!(!redacted.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn test_redact_passthrough() {
        let input = "This is a normal error message with no secrets";
        let redacted = redact(input);
        assert_eq!(redacted, input);
    }

    #[test]
    fn test_redact_multiple_occurrences() {
        let input = "Errors: /Users/alice and /Users/bob and api_key=key1 and secret=key2";
        let redacted = redact(input);
        assert!(!redacted.contains("/Users/alice"));
        assert!(!redacted.contains("/Users/bob"));
        // Both api_key and secret should be redacted
        let redacted_count = redacted.matches("<KEY>=<REDACTED>").count();
        assert_eq!(redacted_count, 2);
    }
}
