**Status:** M17 complete (L0–L2 done + L3 Ollama + L3.5 apply + M6–M17 feedback/style/metrics). See README.md for full milestone matrix.

# Self-Evolving Dev Ecosystem (Superorganism) — Atomic Task List

> This project is a Rust workspace daemon. Every task is self-contained.
> Read it fully, execute each step exactly, run the verification command,
> and only proceed when it passes.
>
> **PREREQUISITE:** Rust stable toolchain must be installed.
> Run: `rustup toolchain install stable` if not already installed.
> Check with: `cargo --version`

Reference docs:
- `PLAN.md` — architecture, capability levels, scenarios
- `IMPLEMENTATION.md` — crate layout, IPC protocol, knowledge store, plugin API

---

## TASK-001: Initialize Rust workspace

**Depends on:** nothing
**Files created:** `Cargo.toml` (workspace), all crate stubs

### Steps

```bash
cd ~/projects/self-evolving-dev-ecosystem
```

Create the root workspace `Cargo.toml`:
```toml
[workspace]
members = [
    "crates/protocol",
    "crates/knowledge",
    "crates/cortex",
    "crates/daemon",
    "crates/client",
]
resolver = "2"

[workspace.dependencies]
tokio = { version = "1.37", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
thiserror = "1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1.8", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
```

Create crate directories:
```bash
mkdir -p crates/protocol/src
mkdir -p crates/knowledge/src
mkdir -p crates/cortex/src
mkdir -p crates/daemon/src/sensors
mkdir -p crates/daemon/src/effectors
mkdir -p crates/client/src
mkdir -p tests/integration
```

Create `crates/protocol/Cargo.toml`:
```toml
[package]
name = "organism-protocol"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
```

Create `crates/knowledge/Cargo.toml`:
```toml
[package]
name = "organism-knowledge"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
tempfile = "3.10"
```

Create `crates/cortex/Cargo.toml`:
```toml
[package]
name = "organism-cortex"
version = "0.1.0"
edition = "2021"

[dependencies]
organism-protocol = { path = "../protocol" }
organism-knowledge = { path = "../knowledge" }
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
```

Create `crates/daemon/Cargo.toml`:
```toml
[package]
name = "organism-daemon"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "organism"
path = "src/main.rs"

[dependencies]
organism-protocol = { path = "../protocol" }
organism-knowledge = { path = "../knowledge" }
organism-cortex = { path = "../cortex" }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
```

Create `crates/client/Cargo.toml`:
```toml
[package]
name = "organism-client"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "organism-cli"
path = "src/main.rs"

[dependencies]
organism-protocol = { path = "../protocol" }
tokio = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

Create minimal `src/lib.rs` stubs:
```bash
echo "// organism-protocol" > crates/protocol/src/lib.rs
echo "// organism-knowledge" > crates/knowledge/src/lib.rs
echo "// organism-cortex" > crates/cortex/src/lib.rs
echo "fn main() { println!(\"organism daemon\"); }" > crates/daemon/src/main.rs
echo "fn main() { println!(\"organism cli\"); }" > crates/client/src/main.rs
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo build 2>&1 | tail -10
echo "Exit: $?"
```
**Done when:** `cargo build` exits with code 0 and both binaries compile.

---

## TASK-002: Implement the protocol crate (message types)

**Depends on:** TASK-001
**Files created:** `crates/protocol/src/lib.rs`, `crates/protocol/src/events.rs`, `crates/protocol/src/messages.rs`

### Steps

Create `crates/protocol/src/events.rs`:
```rust
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Context attached to every event for project identification
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventContext {
    pub project_id: Option<String>,
    pub detected_stack: Option<Vec<String>>,
    pub last_error_signature: Option<String>,
}

/// A command run in a terminal session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalEvent {
    pub ts: DateTime<Utc>,
    pub pid: u32,
    pub cwd: String,
    pub command_line: String,
    pub stdout_snippet: Option<String>,
    pub stderr_snippet: Option<String>,
    /// Keystrokes per minute (0 if not measured)
    pub keystroke_rate: f32,
    pub context: EventContext,
}

/// A file system change event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEvent {
    pub ts: DateTime<Utc>,
    pub path: String,
    pub event_type: FileEventType,
    pub size_bytes: u64,
    pub context: EventContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileEventType {
    Create,
    Modify,
    Delete,
    Rename,
}

/// A git repository event (commit, branch switch, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitEvent {
    pub ts: DateTime<Utc>,
    pub repo_path: String,
    pub branch: String,
    pub head_sha: String,
    pub commit_msg: Option<String>,
    pub author: Option<String>,
    pub context: EventContext,
}

/// A process lifecycle event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessEvent {
    pub ts: DateTime<Utc>,
    pub pid: u32,
    pub cmd: String,
    pub exit_code: Option<i32>,
    pub cpu_ms: u64,
    pub mem_kb: u64,
    pub context: EventContext,
}

/// Union of all event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrganismEvent {
    Terminal(TerminalEvent),
    File(FileEvent),
    Git(GitEvent),
    Process(ProcessEvent),
}

impl OrganismEvent {
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::Terminal(e) => e.ts,
            Self::File(e) => e.ts,
            Self::Git(e) => e.ts,
            Self::Process(e) => e.ts,
        }
    }
}
```

Create `crates/protocol/src/messages.rs`:
```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use crate::events::OrganismEvent;

/// Protocol version. Increment when breaking changes are made.
pub const PROTOCOL_VERSION: u8 = 1;

/// Versioned envelope wrapping all messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u8,
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub id: String,
    pub ts: DateTime<Utc>,
    pub payload: serde_json::Value,
}

impl Envelope {
    pub fn new(msg_type: MessageType, payload: serde_json::Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            msg_type,
            id: Uuid::new_v4().to_string(),
            ts: Utc::now(),
            payload,
        }
    }

    pub fn request(method: &str, params: serde_json::Value) -> Self {
        Self::new(
            MessageType::Request,
            serde_json::json!({ "method": method, "params": params }),
        )
    }

    pub fn ok_response(request_id: &str, result: serde_json::Value) -> Self {
        let mut env = Self::new(MessageType::Response, serde_json::json!({ "result": result }));
        env.id = request_id.to_string();
        env
    }

    pub fn error_response(request_id: &str, message: &str) -> Self {
        let mut env = Self::new(
            MessageType::Error,
            serde_json::json!({ "error": message }),
        );
        env.id = request_id.to_string();
        env
    }

    pub fn event(event: &OrganismEvent) -> anyhow::Result<Self> {
        Ok(Self::new(
            MessageType::Event,
            serde_json::to_value(event)?,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum MessageType {
    Request,
    Response,
    Error,
    Event,
    Heartbeat,
}

/// Status response payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub status: String,
    pub uptime_s: u64,
    pub trust_level: String,
    pub active_sensors: Vec<String>,
    pub events_processed: u64,
}
```

Update `crates/protocol/src/lib.rs`:
```rust
pub mod events;
pub mod messages;

pub use events::*;
pub use messages::*;
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test -p organism-protocol 2>&1 | tail -10
echo "Exit: $?"
```
**Done when:** exits 0 (tests run or no tests yet, just compilation).

---

## TASK-003: Write tests for the protocol crate

**Depends on:** TASK-002
**Files created:** `crates/protocol/src/lib.rs` (tests section added)

### Steps

Add a `tests` module at the bottom of `crates/protocol/src/lib.rs`:
```rust
pub mod events;
pub mod messages;

pub use events::*;
pub use messages::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_envelope_request_roundtrip() {
        let env = Envelope::request("status.get", serde_json::json!({}));
        let serialized = serde_json::to_string(&env).unwrap();
        let deserialized: Envelope = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.v, PROTOCOL_VERSION);
        assert_eq!(deserialized.msg_type, MessageType::Request);
    }

    #[test]
    fn test_terminal_event_serializes() {
        use chrono::Utc;
        let event = OrganismEvent::Terminal(TerminalEvent {
            ts: Utc::now(),
            pid: 1234,
            cwd: "/home/user/projects".to_string(),
            command_line: "cargo build".to_string(),
            stdout_snippet: None,
            stderr_snippet: None,
            keystroke_rate: 0.0,
            context: EventContext::default(),
        });
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("terminal"));
        assert!(json.contains("cargo build"));
    }

    #[test]
    fn test_file_event_roundtrip() {
        use chrono::Utc;
        let event = FileEvent {
            ts: Utc::now(),
            path: "/home/user/src/main.rs".to_string(),
            event_type: FileEventType::Modify,
            size_bytes: 1024,
            context: EventContext::default(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: FileEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "/home/user/src/main.rs");
        assert_eq!(back.event_type, FileEventType::Modify);
    }

    #[test]
    fn test_envelope_event_from_organism_event() {
        use chrono::Utc;
        let event = OrganismEvent::Git(GitEvent {
            ts: Utc::now(),
            repo_path: "/home/user/projects/app".to_string(),
            branch: "main".to_string(),
            head_sha: "abc123".to_string(),
            commit_msg: Some("feat: add feature".to_string()),
            author: Some("dev".to_string()),
            context: EventContext::default(),
        });
        let envelope = Envelope::event(&event).unwrap();
        assert_eq!(envelope.msg_type, MessageType::Event);
    }

    #[test]
    fn test_ok_response_preserves_request_id() {
        let env = Envelope::ok_response("req-123", serde_json::json!({"status": "ok"}));
        assert_eq!(env.id, "req-123");
        assert_eq!(env.msg_type, MessageType::Response);
    }
}
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test -p organism-protocol -- --nocapture 2>&1 | tail -15
```
**Done when:** all 5 tests pass (`test result: ok. 5 passed`).

---

## TASK-004: Implement the knowledge crate (persistent store)

**Depends on:** TASK-001
**Files created:** `crates/knowledge/src/lib.rs`, `crates/knowledge/src/store.rs`, `crates/knowledge/src/types.rs`

### Steps

Create `crates/knowledge/src/types.rs`:
```rust
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A fix record: known error → patch solution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixRecord {
    pub id: String,
    /// sha256 of the error snippet that triggers this fix
    pub signature_hash: String,
    /// Unified diff or description of the fix
    pub patch: String,
    /// 0.0 - 1.0 confidence based on success rate
    pub confidence: f64,
    pub applied_count: u32,
    pub last_applied: DateTime<Utc>,
    /// "learned" | "manual" | "imported"
    pub source: String,
}

/// A detected coding pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternRecord {
    pub id: String,
    pub trigger: String,
    pub action: String,
    pub frequency: u32,
    pub confidence: f64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub examples: Vec<String>,
}

/// A project metadata record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    pub path: String,
    pub name: String,
    pub detected_stack: Vec<String>,
    pub primary_language: Option<String>,
    pub last_accessed: DateTime<Utc>,
    pub session_count: u32,
}

/// Key prefixes for the key-value store
pub mod keys {
    pub const FIX_PREFIX: &str = "fix:";
    pub const PATTERN_PREFIX: &str = "pat:";
    pub const PROJECT_PREFIX: &str = "proj:";
    pub const STATS_PREFIX: &str = "stats:";
}
```

Create `crates/knowledge/src/store.rs`:
```rust
//! File-based key-value store backed by JSON files.
//! Simple and portable — no native dependencies.
//! Suitable for Level 0-2 capability. Swap for RocksDB at Level 3+.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::{FixRecord, PatternRecord, ProjectMeta, keys};

/// File-backed key-value store
pub struct KnowledgeStore {
    data_dir: PathBuf,
    /// In-memory cache: key → JSON string
    cache: HashMap<String, String>,
}

impl KnowledgeStore {
    pub fn open(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)
            .with_context(|| format!("Creating data dir: {:?}", data_dir))?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            cache: HashMap::new(),
        })
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        // Replace ':' and '/' with safe characters for filenames
        let safe_key = key.replace([':', '/'], "_");
        self.data_dir.join(format!("{}.json", safe_key))
    }

    pub fn get<T: for<'de> Deserialize<'de>>(&mut self, key: &str) -> Result<Option<T>> {
        if let Some(cached) = self.cache.get(key) {
            return Ok(Some(serde_json::from_str(cached)?));
        }
        let path = self.key_to_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Reading {:?}", path))?;
        self.cache.insert(key.to_string(), content.clone());
        Ok(Some(serde_json::from_str(&content)?))
    }

    pub fn put<T: Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        let content = serde_json::to_string_pretty(value)?;
        let path = self.key_to_path(key);
        fs::write(&path, &content)
            .with_context(|| format!("Writing {:?}", path))?;
        self.cache.insert(key.to_string(), content);
        Ok(())
    }

    pub fn delete(&mut self, key: &str) -> Result<()> {
        let path = self.key_to_path(key);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        self.cache.remove(key);
        Ok(())
    }

    pub fn list_keys(&self, prefix: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let safe_prefix = prefix.replace([':', '/'], "_");
        for entry in fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let fname = entry.file_name().to_string_lossy().into_owned();
            if fname.starts_with(&safe_prefix) && fname.ends_with(".json") {
                let key = fname
                    .trim_end_matches(".json")
                    .replace('_', ":")
                    .to_string();
                keys.push(key);
            }
        }
        Ok(keys)
    }

    // --- Typed accessors ---

    pub fn get_fix(&mut self, sig_hash: &str) -> Result<Option<FixRecord>> {
        self.get(&format!("{}{}", keys::FIX_PREFIX, sig_hash))
    }

    pub fn put_fix(&mut self, record: &FixRecord) -> Result<()> {
        self.put(&format!("{}{}", keys::FIX_PREFIX, record.signature_hash), record)
    }

    pub fn get_pattern(&mut self, id: &str) -> Result<Option<PatternRecord>> {
        self.get(&format!("{}{}", keys::PATTERN_PREFIX, id))
    }

    pub fn put_pattern(&mut self, record: &PatternRecord) -> Result<()> {
        self.put(&format!("{}{}", keys::PATTERN_PREFIX, record.id), record)
    }

    pub fn list_patterns(&self) -> Result<Vec<String>> {
        self.list_keys(keys::PATTERN_PREFIX)
    }

    pub fn get_project(&mut self, id: &str) -> Result<Option<ProjectMeta>> {
        self.get(&format!("{}{}", keys::PROJECT_PREFIX, id))
    }

    pub fn put_project(&mut self, meta: &ProjectMeta) -> Result<()> {
        self.put(&format!("{}{}", keys::PROJECT_PREFIX, meta.id), meta)
    }
}
```

Update `crates/knowledge/src/lib.rs`:
```rust
pub mod store;
pub mod types;

pub use store::KnowledgeStore;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use chrono::Utc;

    fn make_store() -> (KnowledgeStore, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = KnowledgeStore::open(tmp.path()).unwrap();
        (store, tmp)
    }

    #[test]
    fn test_put_and_get_fix() {
        let (mut store, _tmp) = make_store();
        let fix = FixRecord {
            id: "fix1".to_string(),
            signature_hash: "abc123".to_string(),
            patch: "- old\n+ new".to_string(),
            confidence: 0.9,
            applied_count: 1,
            last_applied: Utc::now(),
            source: "learned".to_string(),
        };
        store.put_fix(&fix).unwrap();
        let retrieved = store.get_fix("abc123").unwrap().unwrap();
        assert_eq!(retrieved.id, "fix1");
        assert!((retrieved.confidence - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let (mut store, _tmp) = make_store();
        let result: Option<FixRecord> = store.get("nonexistent_key").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_put_and_get_pattern() {
        let (mut store, _tmp) = make_store();
        let pattern = PatternRecord {
            id: "pat1".to_string(),
            trigger: "optimizing bundle_size".to_string(),
            action: "enable tree-shaking".to_string(),
            frequency: 3,
            confidence: 0.75,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            examples: vec!["project-a".to_string()],
        };
        store.put_pattern(&pattern).unwrap();
        let retrieved = store.get_pattern("pat1").unwrap().unwrap();
        assert_eq!(retrieved.trigger, "optimizing bundle_size");
        assert_eq!(retrieved.frequency, 3);
    }

    #[test]
    fn test_put_and_get_project() {
        let (mut store, _tmp) = make_store();
        let meta = ProjectMeta {
            id: "proj1".to_string(),
            path: "/home/dev/myapp".to_string(),
            name: "myapp".to_string(),
            detected_stack: vec!["React".to_string(), "TypeScript".to_string()],
            primary_language: Some("TypeScript".to_string()),
            last_accessed: Utc::now(),
            session_count: 5,
        };
        store.put_project(&meta).unwrap();
        let retrieved = store.get_project("proj1").unwrap().unwrap();
        assert_eq!(retrieved.name, "myapp");
        assert_eq!(retrieved.detected_stack.len(), 2);
    }

    #[test]
    fn test_delete_removes_entry() {
        let (mut store, _tmp) = make_store();
        let fix = FixRecord {
            id: "f2".to_string(),
            signature_hash: "del123".to_string(),
            patch: "patch".to_string(),
            confidence: 0.5,
            applied_count: 0,
            last_applied: Utc::now(),
            source: "manual".to_string(),
        };
        store.put_fix(&fix).unwrap();
        store.delete(&format!("{}del123", keys::FIX_PREFIX)).unwrap();
        let result = store.get_fix("del123").unwrap();
        assert!(result.is_none());
    }
}
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test -p organism-knowledge -- --nocapture 2>&1 | tail -15
```
**Done when:** `test result: ok. 5 passed`.

---

## TASK-005: Implement the cortex crate (pattern engine)

**Depends on:** TASK-002, TASK-004
**Files created:** `crates/cortex/src/lib.rs`, `crates/cortex/src/context_detector.rs`, `crates/cortex/src/pattern_engine.rs`

### Steps

Create `crates/cortex/src/context_detector.rs`:
```rust
//! Detects the current project context from events.

use std::path::Path;
use organism_knowledge::ProjectMeta;

/// Stack indicators found in the filesystem
pub struct StackIndicator {
    pub file: &'static str,
    pub stack: &'static str,
}

const STACK_INDICATORS: &[StackIndicator] = &[
    StackIndicator { file: "package.json", stack: "JavaScript" },
    StackIndicator { file: "Cargo.toml", stack: "Rust" },
    StackIndicator { file: "pyproject.toml", stack: "Python" },
    StackIndicator { file: "go.mod", stack: "Go" },
    StackIndicator { file: "Gemfile", stack: "Ruby" },
    StackIndicator { file: "pom.xml", stack: "Java" },
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
```

Create `crates/cortex/src/pattern_engine.rs`:
```rust
//! Lightweight pattern detection from event streams.
//! Uses simple frequency counting — no external ML dependencies.

use std::collections::HashMap;
use organism_knowledge::{PatternRecord};
use chrono::Utc;
use uuid::Uuid;

/// A simplified event record for pattern mining
#[derive(Debug, Clone)]
pub struct EventRecord {
    pub project_id: String,
    pub event_type: String,
    pub description: String,
}

/// Detect patterns: when event_type X, action Y is always taken next.
/// Returns patterns with frequency >= min_frequency.
pub fn detect_patterns(
    events: &[EventRecord],
    min_frequency: u32,
) -> Vec<PatternRecord> {
    // Count (event_type, next_description) pairs
    let mut pair_counts: HashMap<(String, String), u32> = HashMap::new();

    for window in events.windows(2) {
        let trigger = window[0].event_type.clone();
        let action = window[1].description.clone();
        *pair_counts.entry((trigger, action)).or_insert(0) += 1;
    }

    let now = Utc::now();
    pair_counts
        .into_iter()
        .filter(|(_, count)| *count >= min_frequency)
        .map(|((trigger, action), count)| PatternRecord {
            id: Uuid::new_v4().to_string(),
            trigger,
            action,
            frequency: count,
            confidence: (count as f64 / 10.0).min(1.0),
            first_seen: now,
            last_seen: now,
            examples: Vec::new(),
        })
        .collect()
}

/// Calculate confidence from improvement vs noise floor
pub fn calculate_confidence(improvement: f64, noise_floor: f64) -> f64 {
    if noise_floor == 0.0 {
        return if improvement > 0.0 { 1.0 } else { 0.0 };
    }
    (improvement / noise_floor / 3.0).clamp(0.0, 1.0)
}
```

Update `crates/cortex/src/lib.rs`:
```rust
pub mod context_detector;
pub mod pattern_engine;

pub use context_detector::*;
pub use pattern_engine::*;

#[cfg(test)]
mod tests {
    use super::*;
    use pattern_engine::{detect_patterns, EventRecord, calculate_confidence};

    fn make_event(project: &str, etype: &str, desc: &str) -> EventRecord {
        EventRecord {
            project_id: project.to_string(),
            event_type: etype.to_string(),
            description: desc.to_string(),
        }
    }

    #[test]
    fn test_detect_patterns_frequency_threshold() {
        let events = vec![
            make_event("p1", "build_error", "try cargo fix"),
            make_event("p1", "build_error", "try cargo fix"),
            make_event("p1", "build_error", "try cargo fix"),
            make_event("p1", "lint_warning", "run ruff fix"),
        ];
        let patterns = detect_patterns(&events, 2);
        // The "build_error → try cargo fix" pair appears 2 times in windows
        assert!(!patterns.is_empty());
        let found = patterns.iter().any(|p|
            p.trigger == "build_error" && p.action == "try cargo fix"
        );
        assert!(found, "Expected build_error pattern");
    }

    #[test]
    fn test_detect_patterns_below_threshold_excluded() {
        let events = vec![
            make_event("p1", "rare_event", "rare_action"),
            make_event("p1", "other_event", "other_action"),
        ];
        // With min_frequency=3, nothing should be returned
        let patterns = detect_patterns(&events, 3);
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_calculate_confidence_no_noise() {
        assert_eq!(calculate_confidence(5.0, 0.0), 1.0);
        assert_eq!(calculate_confidence(0.0, 0.0), 0.0);
    }

    #[test]
    fn test_calculate_confidence_clamped() {
        let c = calculate_confidence(100.0, 1.0);
        assert!(c <= 1.0);
        let c2 = calculate_confidence(0.0, 5.0);
        assert!(c2 >= 0.0);
    }

    #[test]
    fn test_context_detector_nonexistent_path() {
        let meta = context_detector::detect_project("/this/does/not/exist");
        assert!(meta.is_none());
    }
}
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test -p organism-cortex -- --nocapture 2>&1 | tail -15
```
**Done when:** `test result: ok. 5 passed`.

---

## TASK-006: Implement the daemon event bus and basic skeleton

**Depends on:** TASK-002, TASK-004, TASK-005
**Files created:** `crates/daemon/src/main.rs`, `crates/daemon/src/event_bus.rs`, `crates/daemon/src/daemon.rs`

### Steps

Create `crates/daemon/src/event_bus.rs`:
```rust
//! Async pub/sub event bus for the daemon.
//! Producers push events; subscribers receive them via broadcast channels.

use tokio::sync::broadcast;
use organism_protocol::OrganismEvent;

pub struct EventBus {
    sender: broadcast::Sender<OrganismEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publish an event to all subscribers.
    /// Returns the number of receivers that got the message.
    pub fn publish(&self, event: OrganismEvent) -> usize {
        self.sender.send(event).unwrap_or(0)
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<OrganismEvent> {
        self.sender.subscribe()
    }
}
```

Create `crates/daemon/src/daemon.rs`:
```rust
//! Core daemon: lifecycle management, sensor orchestration, event routing.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};

use organism_knowledge::KnowledgeStore;
use organism_cortex::{detect_patterns, EventRecord};

use crate::event_bus::EventBus;

#[derive(Debug, Clone, PartialEq)]
pub enum TrustLevel {
    Observer,
    Ask,
    Assist,
    Autonomous,
    Uncaged,
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Observer => write!(f, "observer"),
            Self::Ask => write!(f, "ask"),
            Self::Assist => write!(f, "assist"),
            Self::Autonomous => write!(f, "autonomous"),
            Self::Uncaged => write!(f, "uncaged"),
        }
    }
}

pub struct DaemonState {
    pub trust_level: TrustLevel,
    pub events_processed: u64,
    pub started_at: Instant,
    pub active_sensors: Vec<String>,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            trust_level: TrustLevel::Ask,
            events_processed: 0,
            started_at: Instant::now(),
            active_sensors: vec!["terminal".to_string(), "filesystem".to_string()],
        }
    }

    pub fn uptime_s(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}

pub struct Daemon {
    pub bus: Arc<EventBus>,
    pub state: Arc<RwLock<DaemonState>>,
    pub knowledge: Arc<RwLock<KnowledgeStore>>,
}

impl Daemon {
    pub fn new(knowledge_dir: std::path::PathBuf) -> anyhow::Result<Self> {
        let knowledge = KnowledgeStore::open(&knowledge_dir)?;
        Ok(Self {
            bus: Arc::new(EventBus::new(1024)),
            state: Arc::new(RwLock::new(DaemonState::new())),
            knowledge: Arc::new(RwLock::new(knowledge)),
        })
    }

    /// Main event processing loop: subscribe to bus, process events.
    pub async fn run_event_loop(&self) {
        let mut rx = self.bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let mut state = self.state.write().await;
                    state.events_processed += 1;
                    drop(state);
                    // Future: route to cortex, effectors
                    tracing::debug!(?event, "event received");
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Event bus lagged by {} messages", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!("Event bus closed, stopping event loop");
                    break;
                }
            }
        }
    }
}
```

Update `crates/daemon/src/main.rs`:
```rust
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod event_bus;
mod daemon;

use daemon::Daemon;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize structured logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    info!("🤖 Organism daemon starting...");

    // Determine data directory
    let data_dir = dirs_home_knowledge_dir();
    std::fs::create_dir_all(&data_dir)?;
    info!("Knowledge dir: {:?}", data_dir);

    let daemon = Daemon::new(data_dir)?;
    
    {
        let state = daemon.state.read().await;
        info!(
            trust_level = %state.trust_level,
            sensors = ?state.active_sensors,
            "Daemon ready"
        );
    }

    // Run event loop (keeps daemon alive)
    daemon.run_event_loop().await;

    Ok(())
}

fn dirs_home_knowledge_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    home.join(".organism")
}
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo build -p organism-daemon 2>&1 | tail -10
echo "Exit: $?"
```
**Done when:** exits 0 (daemon binary compiles).

---

## TASK-007: Implement the CLI client skeleton

**Depends on:** TASK-002, TASK-006
**Files created:** `crates/client/src/main.rs`

### Steps

Update `crates/client/src/main.rs`:
```rust
//! Organism CLI — talks to the running daemon.
//! Currently prints status and simulates commands.
//! Future: connect to daemon via Unix socket.

use std::process;
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "status" => cmd_status(),
        "suggest" => cmd_suggest(),
        "log" => cmd_log(),
        "sleep" => cmd_sleep(),
        "wake" => cmd_wake(),
        "--help" | "help" | _ => cmd_help(),
    }
}

fn cmd_help() {
    println!("🤖 Organism CLI");
    println!();
    println!("USAGE:");
    println!("  organism-cli <command>");
    println!();
    println!("COMMANDS:");
    println!("  status    Show daemon status");
    println!("  suggest   Request a suggestion for current directory");
    println!("  log       Show recent daemon activity");
    println!("  sleep     Pause all daemon activity");
    println!("  wake      Resume daemon activity");
    println!("  help      Show this help");
    println!();
    println!("NOTE: Full IPC implementation in progress.");
    println!("      Daemon must be running: organism");
}

fn cmd_status() {
    // Future: connect to Unix socket and call status.get RPC
    println!("🤖 Organism Status");
    println!("  (IPC connection not yet implemented)");
    println!("  Start daemon: organism");
}

fn cmd_suggest() {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    println!("💡 Suggestion for: {}", cwd);
    println!("  (Pattern analysis not yet connected to daemon)");
}

fn cmd_log() {
    let log_path = dirs_home().join(".organism").join("daemon.log");
    if log_path.exists() {
        println!("Recent log entries from {:?}:", log_path);
    } else {
        println!("No log file found at {:?}", log_path);
        println!("Start daemon first: organism");
    }
}

fn cmd_sleep() {
    let lock_path = dirs_home().join(".organism").join("sleep.lock");
    std::fs::create_dir_all(lock_path.parent().unwrap()).ok();
    std::fs::write(&lock_path, b"sleeping").ok();
    println!("😴 Organism paused (sleep.lock created)");
}

fn cmd_wake() {
    let lock_path = dirs_home().join(".organism").join("sleep.lock");
    if lock_path.exists() {
        std::fs::remove_file(&lock_path).ok();
        println!("🟢 Organism resumed");
    } else {
        println!("Organism was not sleeping");
    }
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
}
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo build -p organism-client 2>&1 | tail -5
cargo run -p organism-client -- help 2>/dev/null
echo "Exit: $?"
```
**Done when:** `help` prints usage and exits 0.

---

## TASK-008: Write workspace-level integration tests

**Depends on:** TASK-003, TASK-004, TASK-005
**Files created:** `tests/integration/test_event_flow.rs`, `tests/integration/main.rs`

### Steps

Add `[[test]]` to workspace `Cargo.toml` — actually Rust integration tests live in the crate's `tests/` directory. Let's add them to the daemon crate.

Create `crates/daemon/tests/integration_test.rs`:
```rust
//! Integration tests for the daemon event bus.

use std::sync::Arc;
use tokio::sync::RwLock;
use organism_protocol::{OrganismEvent, TerminalEvent, EventContext};
use chrono::Utc;

// We import internal modules by re-exporting them or using #[path]
// For simplicity, we replicate the EventBus here for testing.
// Real integration tests would use the daemon library (convert daemon to lib + bin).

mod event_bus {
    use tokio::sync::broadcast;
    use organism_protocol::OrganismEvent;

    pub struct EventBus {
        sender: broadcast::Sender<OrganismEvent>,
    }

    impl EventBus {
        pub fn new(capacity: usize) -> Self {
            let (sender, _) = broadcast::channel(capacity);
            Self { sender }
        }
        pub fn publish(&self, event: OrganismEvent) -> usize {
            self.sender.send(event).unwrap_or(0)
        }
        pub fn subscribe(&self) -> broadcast::Receiver<OrganismEvent> {
            self.sender.subscribe()
        }
    }
}

use event_bus::EventBus;

#[tokio::test]
async fn test_event_bus_publish_subscribe() {
    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();

    let event = OrganismEvent::Terminal(TerminalEvent {
        ts: Utc::now(),
        pid: 9999,
        cwd: "/test".to_string(),
        command_line: "cargo test".to_string(),
        stdout_snippet: None,
        stderr_snippet: None,
        keystroke_rate: 0.0,
        context: EventContext::default(),
    });

    bus.publish(event);

    let received = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        rx.recv(),
    ).await.expect("timeout").expect("recv error");

    if let OrganismEvent::Terminal(t) = received {
        assert_eq!(t.command_line, "cargo test");
        assert_eq!(t.pid, 9999);
    } else {
        panic!("Expected terminal event");
    }
}

#[tokio::test]
async fn test_event_bus_multiple_subscribers() {
    let bus = EventBus::new(64);
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    let event = OrganismEvent::Terminal(TerminalEvent {
        ts: Utc::now(),
        pid: 1,
        cwd: "/".to_string(),
        command_line: "ls".to_string(),
        stdout_snippet: None,
        stderr_snippet: None,
        keystroke_rate: 0.0,
        context: EventContext::default(),
    });

    bus.publish(event);

    let r1 = rx1.recv().await.unwrap();
    let r2 = rx2.recv().await.unwrap();

    if let (OrganismEvent::Terminal(t1), OrganismEvent::Terminal(t2)) = (r1, r2) {
        assert_eq!(t1.command_line, t2.command_line);
    }
}

#[tokio::test]
async fn test_knowledge_store_in_tempdir() {
    use organism_knowledge::{KnowledgeStore, FixRecord};
    use chrono::Utc;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut store = KnowledgeStore::open(tmp.path()).unwrap();

    let fix = FixRecord {
        id: "integration-fix".to_string(),
        signature_hash: "hash999".to_string(),
        patch: "apply fix X".to_string(),
        confidence: 0.88,
        applied_count: 2,
        last_applied: Utc::now(),
        source: "learned".to_string(),
    };

    store.put_fix(&fix).unwrap();
    let retrieved = store.get_fix("hash999").unwrap().unwrap();
    assert_eq!(retrieved.id, "integration-fix");
    assert!((retrieved.confidence - 0.88).abs() < 0.001);
}
```

Add to `crates/daemon/Cargo.toml` under `[dev-dependencies]`:
```toml
[dev-dependencies]
tempfile = "3.10"
tokio = { workspace = true }
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test -p organism-daemon -- --nocapture 2>&1 | tail -20
```
**Done when:** all integration tests pass.

---

## TASK-009: Add GitHub Actions CI and .gitignore

**Depends on:** TASK-008
**Files created:** `.github/workflows/ci.yml`, `.gitignore`

### Steps

Create `.gitignore`:
```
/target/
**/*.rs.bk
Cargo.lock
.DS_Store
~/.organism/
*.log
```

Create `.github/workflows/ci.yml`:
```yaml
name: Superorganism CI
on: [push, pull_request]

jobs:
  build-and-test:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        rust: [stable]
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
      - name: Cache cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target/
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
      - name: Build all crates
        run: cargo build --workspace --all-features
      - name: Run all tests
        run: cargo test --workspace --all-features -- --nocapture

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - run: cargo clippy --workspace --all-features -- -D warnings
```

Run clippy locally and fix warnings:
```bash
cd ~/projects/self-evolving-dev-ecosystem
rustup component add clippy
cargo clippy --workspace -- -D warnings 2>&1 | head -40
```

Fix any clippy warnings reported. Common fixes:
- `dead_code` warnings → add `#[allow(dead_code)]` or remove unused items
- `unused_variables` → prefix with `_` if intentional

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test --workspace -- --nocapture 2>&1 | grep -E "test result|FAILED|error"
```
**Done when:** all test suites show `test result: ok` with no FAILED.

---

## TASK-010: Write README and run full workspace test

**Depends on:** TASK-009
**Files created:** `README.md`

### Steps

Create `README.md`:
```markdown
# 🧬 Self-Evolving Dev Ecosystem (Organism)

> A local daemon that learns how you work, anticipates your next move,
> and eventually codes alongside you as a digital twin.

## Status: Level 0 — Observer

The daemon currently provides:
- Event bus (pub/sub) for terminal, file, git, and process events
- File-backed knowledge store (patterns, fixes, project metadata)
- Pattern detection from event streams
- Context detection from project filesystem
- CLI client skeleton (`organism-cli`)

## Prerequisites

- Rust stable toolchain: `rustup toolchain install stable`

## Build

```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo build --workspace --release
```

## Run

```bash
# Start daemon
./target/release/organism

# In another terminal, use CLI
./target/release/organism-cli status
./target/release/organism-cli help
./target/release/organism-cli sleep
./target/release/organism-cli wake
```

## Test

```bash
cargo test --workspace
```

## Architecture

See `PLAN.md` and `IMPLEMENTATION.md` for full architecture details.

### Crates

| Crate | Purpose |
|-------|---------|
| `organism-protocol` | Message types, event structs, IPC envelope |
| `organism-knowledge` | File-backed KV store for patterns and fixes |
| `organism-cortex` | Pattern detection, context detection |
| `organism-daemon` | Main daemon binary with event bus |
| `organism-client` | `organism-cli` command-line interface |

## Capability Roadmap

- **Level 0 (now):** Event bus, knowledge store, pattern detection skeleton
- **Level 1:** Error detection + fix database + inline suggestions
- **Level 2:** Style learning + code generation in your voice
- **Level 3:** Silent background actions (format, cache, security scan)
- **Level 4:** Full digital twin — generates code that looks like you

## Data Location

All data stored in `~/.organism/` — nothing ever leaves your machine.
```

Run full test suite:
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test --workspace 2>&1 | tee test_results.txt
cat test_results.txt | grep -E "test result|error\[|^error"
```

### Verification
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test --workspace 2>&1 | grep "test result"
cargo build --workspace --release 2>&1 | tail -5
./target/release/organism-cli help
```
**Done when:**
- All `test result:` lines show `ok`
- Release build succeeds
- `organism-cli help` prints usage

---

## Summary

| Phase | Tasks | Goal |
|-------|-------|------|
| Workspace | 001 | Cargo workspace with 5 crates |
| Protocol | 002-003 | Typed events, IPC envelope, roundtrip tests |
| Knowledge | 004 | File-backed KV store with typed accessors |
| Cortex | 005 | Context detection, pattern engine |
| Daemon | 006-007 | Async event bus, daemon skeleton, CLI |
| Integration | 008-009 | Integration tests, CI workflow |
| Polish | 010 | README, full test run, release build |

## What's NOT implemented yet (for Level 1+)

These are the next phases after this TASKS.md is complete:

- **Unix socket IPC** — `organism-cli` talking to running daemon
- **Terminal sensor** — zsh `preexec`/`precmd` hook integration
- **File watcher** — `notify` crate integration for real-time file events
- **Error classifier** — parse stderr patterns into known error signatures
- **Ollama integration** — local LLM for code suggestion generation
- **Shell install script** — `organism-install` that configures zsh hooks
- **macOS LaunchAgent** — auto-start daemon on login
