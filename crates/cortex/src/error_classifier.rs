//! Classifies command-line errors into a stable signature.
//!
//! Pattern-matches stderr against well-known error formats (rustc, npm, python,
//! shell). Produces an `ErrorSignature` with a deterministic hash suitable for
//! deduplicating recurring errors.
//!
//! NOTE: Hash uses `std::collections::hash_map::DefaultHasher`. This is
//! deterministic within a single process (no random seed for the algorithm
//! itself across versions, but Rust does not guarantee stability across
//! compiler/std versions). For session-local dedup this is sufficient. Swap for
//! `sha2` if cross-process / cross-version stability becomes required.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorSignature {
    /// "rustc" | "cargo" | "npm" | "python" | "shell" | "unknown"
    pub tool: String,
    /// e.g. "E0599" | "ModuleNotFoundError" | "command_not_found"
    pub kind: String,
    /// hex digest of (tool|kind|normalized_message)
    pub hash: String,
    /// First ~200 chars of stderr matched
    pub raw_excerpt: String,
}

fn hash_signature(tool: &str, kind: &str, normalized: &str) -> String {
    let mut hasher = DefaultHasher::new();
    tool.hash(&mut hasher);
    kind.hash(&mut hasher);
    normalized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn excerpt(s: &str) -> String {
    s.chars().take(200).collect()
}

fn first_64(s: &str) -> String {
    s.chars().take(64).collect()
}

/// Classify the outcome of a command into an `ErrorSignature`.
/// Returns `None` if the command succeeded or there's no useful signal.
pub fn classify(
    cmd: &str,
    exit_code: Option<i32>,
    stderr: Option<&str>,
) -> Option<ErrorSignature> {
    if exit_code == Some(0) {
        return None;
    }

    let stderr_str = stderr.unwrap_or("");

    // Rule 1: rustc error[E####]
    let rustc_re = Regex::new(r"error\[E(\d+)\]").unwrap();
    if let Some(caps) = rustc_re.captures(stderr_str) {
        let kind = format!("E{}", &caps[1]);
        let raw = excerpt(stderr_str);
        let normalized = first_64(stderr_str);
        let hash = hash_signature("rustc", &kind, &normalized);
        return Some(ErrorSignature {
            tool: "rustc".to_string(),
            kind,
            hash,
            raw_excerpt: raw,
        });
    }

    // Rule 2: npm ERR!
    let npm_re = Regex::new(r"(?m)^npm ERR!").unwrap();
    if npm_re.is_match(stderr_str) {
        // Find the first npm ERR! line and extract the next non-empty content
        // after the "npm ERR!" prefix on that line, falling back to the
        // following non-empty line.
        let mut kind = String::new();
        for line in stderr_str.lines() {
            if let Some(rest) = line.strip_prefix("npm ERR!") {
                let trimmed = rest.trim();
                if !trimmed.is_empty() {
                    kind = trimmed.chars().take(80).collect();
                    break;
                }
            }
        }
        if kind.is_empty() {
            kind = "unknown".to_string();
        }
        let raw = excerpt(stderr_str);
        let normalized = first_64(stderr_str);
        let hash = hash_signature("npm", &kind, &normalized);
        return Some(ErrorSignature {
            tool: "npm".to_string(),
            kind,
            hash,
            raw_excerpt: raw,
        });
    }

    // Rule 3: Python traceback followed by ErrorClass:
    let traceback_re = Regex::new(r"(?m)^Traceback").unwrap();
    if traceback_re.is_match(stderr_str) {
        let err_re = Regex::new(r"(?m)^([A-Za-z_][A-Za-z0-9_]*Error):").unwrap();
        if let Some(caps) = err_re.captures(stderr_str) {
            let kind = caps[1].to_string();
            let raw = excerpt(stderr_str);
            let normalized = first_64(stderr_str);
            let hash = hash_signature("python", &kind, &normalized);
            return Some(ErrorSignature {
                tool: "python".to_string(),
                kind,
                hash,
                raw_excerpt: raw,
            });
        }
    }

    // Rule 4: shell command not found
    if stderr_str.contains("command not found") {
        let kind = "command_not_found".to_string();
        let raw = excerpt(stderr_str);
        let normalized = first_64(stderr_str);
        let hash = hash_signature("shell", &kind, &normalized);
        return Some(ErrorSignature {
            tool: "shell".to_string(),
            kind,
            hash,
            raw_excerpt: raw,
        });
    }

    // Rule 5: nonzero exit, no other match
    if let Some(code) = exit_code {
        if code != 0 {
            let kind = format!("exit_{}", code);
            let raw = excerpt(if stderr_str.is_empty() { cmd } else { stderr_str });
            let normalized = first_64(if stderr_str.is_empty() { cmd } else { stderr_str });
            let hash = hash_signature("unknown", &kind, &normalized);
            return Some(ErrorSignature {
                tool: "unknown".to_string(),
                kind,
                hash,
                raw_excerpt: raw,
            });
        }
    }

    // exit_code None and no stderr signal
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rustc_e0599_classified() {
        let stderr = "error[E0599]: no method named `foo` found for type `Bar`";
        let sig = classify("cargo build", Some(101), Some(stderr)).unwrap();
        assert_eq!(sig.tool, "rustc");
        assert_eq!(sig.kind, "E0599");
        assert!(!sig.hash.is_empty());
    }

    #[test]
    fn test_npm_err_classified() {
        let stderr = "npm ERR! code ENOENT\nnpm ERR! syscall open\nnpm ERR! path /tmp/missing";
        let sig = classify("npm install", Some(1), Some(stderr)).unwrap();
        assert_eq!(sig.tool, "npm");
        assert!(sig.kind.contains("ENOENT") || sig.kind.contains("code"));
    }

    #[test]
    fn test_python_traceback_classified() {
        let stderr = "Traceback (most recent call last):\n  File \"x.py\", line 1, in <module>\n    import nope\nModuleNotFoundError: No module named 'nope'";
        let sig = classify("python x.py", Some(1), Some(stderr)).unwrap();
        assert_eq!(sig.tool, "python");
        assert_eq!(sig.kind, "ModuleNotFoundError");
    }

    #[test]
    fn test_command_not_found_classified() {
        let stderr = "zsh: command not found: foozle";
        let sig = classify("foozle", Some(127), Some(stderr)).unwrap();
        assert_eq!(sig.tool, "shell");
        assert_eq!(sig.kind, "command_not_found");
    }

    #[test]
    fn test_zero_exit_returns_none() {
        let sig = classify("ls", Some(0), Some("anything"));
        assert!(sig.is_none());
    }

    #[test]
    fn test_unknown_exit_code_classifies_as_unknown() {
        let sig = classify("./mything", Some(42), Some("")).unwrap();
        assert_eq!(sig.tool, "unknown");
        assert_eq!(sig.kind, "exit_42");
    }

    #[test]
    fn test_hash_is_deterministic() {
        let stderr = "error[E0599]: no method named `foo`";
        let a = classify("cargo build", Some(101), Some(stderr)).unwrap();
        let b = classify("cargo build", Some(101), Some(stderr)).unwrap();
        assert_eq!(a.hash, b.hash);
    }

    #[test]
    fn test_hash_differs_for_different_kinds() {
        let a = classify("cargo build", Some(101), Some("error[E0599]: x")).unwrap();
        let b = classify("cargo build", Some(101), Some("error[E0277]: x")).unwrap();
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn test_no_exit_no_stderr_returns_none() {
        let sig = classify("ls", None, None);
        assert!(sig.is_none());
    }
}
