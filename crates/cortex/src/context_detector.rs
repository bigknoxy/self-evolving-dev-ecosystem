//! Detects the current project context from events.

use organism_knowledge::ProjectMeta;
use std::path::Path;

/// Stack indicators found in the filesystem
pub struct StackIndicator {
    pub file: &'static str,
    pub stack: &'static str,
}

const STACK_INDICATORS: &[StackIndicator] = &[
    StackIndicator {
        file: "package.json",
        stack: "JavaScript",
    },
    StackIndicator {
        file: "Cargo.toml",
        stack: "Rust",
    },
    StackIndicator {
        file: "pyproject.toml",
        stack: "Python",
    },
    StackIndicator {
        file: "go.mod",
        stack: "Go",
    },
    StackIndicator {
        file: "Gemfile",
        stack: "Ruby",
    },
    StackIndicator {
        file: "pom.xml",
        stack: "Java",
    },
];

/// Detect the stack for a given project directory
pub fn detect_stack(project_path: &str) -> Vec<String> {
    let path = Path::new(project_path);
    let mut stack = Vec::new();
    for indicator in STACK_INDICATORS {
        if path.join(indicator.file).exists() {
            stack.push(indicator.stack.to_string());
        }
    }
    stack
}

/// Build a ProjectMeta from a filesystem path
pub fn detect_project(project_path: &str) -> Option<ProjectMeta> {
    let path = Path::new(project_path);
    if !path.exists() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy().into_owned();
    let stack = detect_stack(project_path);
    let primary_language = stack.first().cloned();
    Some(ProjectMeta {
        id: hex_id(project_path),
        path: project_path.to_string(),
        name,
        detected_stack: stack,
        primary_language,
        last_accessed: chrono::Utc::now(),
        session_count: 1,
    })
}

fn hex_id(input: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    input.hash(&mut h);
    format!("{:016x}", h.finish())
}
