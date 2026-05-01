//! Parse cached LLM suggestion text into an actionable ApplyPlan.
//! Pure, no I/O, no LLM calls.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApplyPlan {
    Patch { diff: String },
    Shell { command: String },
    Note { text: String },
}

fn fence_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?s)```([a-zA-Z]*)\n(.*?)```").expect("fence regex is a compile-time literal")
    })
}

pub fn extract_plan(suggestion: &str) -> ApplyPlan {
    for caps in fence_re().captures_iter(suggestion) {
        let lang = caps
            .get(1)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();
        let body = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        match lang.as_str() {
            "diff" | "patch" => {
                return ApplyPlan::Patch {
                    diff: body.to_string(),
                }
            }
            "bash" | "sh" | "zsh" | "shell" => {
                return ApplyPlan::Shell {
                    command: body.to_string(),
                }
            }
            _ => continue,
        }
    }
    ApplyPlan::Note {
        text: suggestion.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_block_extracted() {
        let input = "foo\n```diff\n-a\n+b\n```\nbar";
        assert_eq!(
            extract_plan(input),
            ApplyPlan::Patch {
                diff: "-a\n+b\n".to_string()
            }
        );
    }

    #[test]
    fn patch_block_extracted() {
        let input = "```patch\nhello\n```";
        assert_eq!(
            extract_plan(input),
            ApplyPlan::Patch {
                diff: "hello\n".to_string()
            }
        );
    }

    #[test]
    fn bash_block_extracted() {
        let input = "```bash\necho hi\n```";
        assert_eq!(
            extract_plan(input),
            ApplyPlan::Shell {
                command: "echo hi\n".to_string()
            }
        );
    }

    #[test]
    fn sh_block_extracted() {
        let input = "```sh\nls\n```";
        assert_eq!(
            extract_plan(input),
            ApplyPlan::Shell {
                command: "ls\n".to_string()
            }
        );
    }

    #[test]
    fn unknown_lang_falls_through() {
        let input = "```python\nprint(1)\n```";
        assert_eq!(
            extract_plan(input),
            ApplyPlan::Note {
                text: "```python\nprint(1)\n```".to_string()
            }
        );
    }

    #[test]
    fn no_block_returns_note() {
        let input = "just text";
        assert_eq!(
            extract_plan(input),
            ApplyPlan::Note {
                text: "just text".to_string()
            }
        );
    }

    #[test]
    fn first_block_wins() {
        let input = "```bash\necho hi\n```\n```diff\n-a\n+b\n```";
        assert_eq!(
            extract_plan(input),
            ApplyPlan::Shell {
                command: "echo hi\n".to_string()
            }
        );
    }
}
