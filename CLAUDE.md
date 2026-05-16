# Self-Evolving Dev Ecosystem — Claude Code Project Instructions

## Agentic Workflow
Follow the **Agentic Workflow & Resource Management Policies** in `~/.claude/CLAUDE.md` (subagent decomposition, Ollama MCP for local refactor/lint/docs/summarization, sequential synthesis). Project orchestration plan: `EXECUTION-PLAN.md`.

## Project Overview
Rust workspace daemon (6 crates). Learns developer patterns. Uses Tokio async runtime.
**Current state: M17 complete** — error classification, Ollama suggestions, feedback loop, StyleProfile, apply workflow, metrics/baseline, notification gates, post-apply prompt.

## Before Starting
1. Read `AGENTS.md` — crate dependency order, Rust gotchas, toolchain requirements, M17 Definition of Done.
2. Read `IMPLEMENTATION.md` — crate layout, IPC protocol, knowledge schema, Ollama integration, apply workflow.
3. Read `LEARNINGS.md` — per-milestone gotchas and non-obvious discoveries.

## Prerequisites
```bash
cargo --version    # must succeed
rustup component add clippy
```

## Allowed Shell Commands

```bash
# Build
cargo build --workspace
cargo build --workspace --release
cargo build -p organism-protocol
cargo build -p organism-knowledge
cargo build -p organism-cortex
cargo build -p organism-daemon
cargo build -p organism-client

# Test
cargo test --workspace
cargo test --workspace -- --nocapture
cargo test -p organism-protocol
cargo test -p organism-knowledge
cargo test -p organism-cortex
cargo test -p organism-daemon
cargo test -p organism-client
cargo test -p organism-ollama

# Lint
cargo clippy --workspace -- -D warnings
cargo clippy --workspace

# Run
cargo run -p organism-daemon
cargo run -p organism-client -- help
cargo run -p organism-client -- status
./target/release/organism-cli help

# Inspect
cargo tree
cargo check --workspace
```

## Workspace Structure

```
Cargo.toml                    — workspace manifest (shared deps here)
crates/
  protocol/                   — message types, event structs (no internal deps)
  knowledge/                  — file-backed KV store (no internal deps)
  cortex/                     — pattern engine (depends on protocol, knowledge)
  ollama/                     — Ollama HTTP client (depends on protocol)
  daemon/                     — main binary (depends on all)
  client/                     — CLI binary (depends on protocol)
```

**Build order matters**: always build `protocol` and `knowledge` before `cortex` before `daemon`.

## Rust Code Standards

- `anyhow::Result` for error propagation in binary crates
- `thiserror` for error types in library crates  
- `#[derive(Debug, Clone, Serialize, Deserialize)]` on all data types
- `Arc<RwLock<T>>` for shared state in async context
- `#[cfg(test)] mod tests { ... }` at end of each `src/*.rs` file
- `tempfile::TempDir` for test filesystem isolation

## Serde Requirements

All types in `organism-protocol` must:
1. Implement `Serialize` + `Deserialize`
2. Roundtrip test: `serialize → str → deserialize → original`
3. Use `#[serde(tag = "kind", rename_all = "snake_case")]` on event enums
4. Exception: `Verdict` enum — do NOT add `rename_all`; on-disk format is PascalCase (`"Accepted"`, `"Rejected"`, `"Ignored"`, `"Applied"`). Wire format lowercased manually in `ipc.rs`.
5. Use `#[serde(default)]` on new fields added to existing persisted structs (backward compat)

## Testing Requirements

- Every `pub` function gets at least one test
- Error paths are tested (file not found, bad input, etc.)
- Async tests use `#[tokio::test]`
- Sync tests use `#[test]`
- Knowledge store tests always use `tempfile::TempDir`

## Clippy Policy

`cargo clippy -- -D warnings` must pass before any task is marked complete.
Fix warnings — never suppress with `#[allow(...)]` unless unavoidable.
Acceptable suppressions (must be justified in comment):
- `#[allow(dead_code)]` on public API stubs not yet called

## Learnings Protocol

After completing each task:
```bash
echo "## Completed: TASK-NNN — $(date +%Y-%m-%d)" >> LEARNINGS.md
```

After any compiler/clippy error that took > 5 minutes to fix:
Write a full learning entry in `LEARNINGS.md`.

## Do NOT

- Add `rocksdb` crate (requires C++ compilation — use file-based store)
- Use `std::thread::spawn()` in async code (use `tokio::spawn()`)
- Store `broadcast::Receiver` in shared state (it's not Clone)
- Use `unwrap()` outside of test code
- Add version numbers to crate deps if they're in `[workspace.dependencies]`
- Access `~/.organism/` in tests (use `tempfile::TempDir`)
- Add `rename_all = "snake_case"` to `Verdict` (breaks existing on-disk data)
- Forget `use std::io::IsTerminal` when calling `.is_terminal()` on stdin — must import trait explicitly
- Omit `#[serde(default)]` on new fields in persisted structs (old JSON missing field = deserialization error)
- Run tests using global `OnceLock` state concurrently — add `#[serial]` from `serial_test` crate
