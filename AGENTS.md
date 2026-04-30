# AGENTS.md — Self-Evolving Dev Ecosystem (Organism)

> Instructions for all AI coding agents working on this project.
> Read this file **before** reading any other file.

---

## Project at a Glance

| Item | Value |
|------|-------|
| Language | Rust (stable toolchain) |
| Workspace | Cargo workspace with 5 crates |
| Async runtime | Tokio |
| Build tool | `cargo` |
| Lint | `clippy` |
| Test | `cargo test --workspace` |

---

## Prerequisite Check

Before starting any task:
```bash
rustup toolchain install stable
cargo --version   # must be >= 1.70.0
rustup component add clippy
```

If Rust is not installed:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

---

## Golden Rule

**Never mark a task complete unless its verification command exits 0.**
`cargo build` success alone is not sufficient — `cargo test` must also pass.

---

## Workflow Protocol

1. Read `TASKS.md` — task order (TASK-001 → TASK-010).
2. For each task:
   a. Read the full task before writing any code.
   b. Execute every step in order.
   c. Run the `### Verification` command.
   d. Pass → move on. Fail → fix and re-run.
3. After every 3 tasks:
   ```bash
   cd ~/projects/self-evolving-dev-ecosystem
   cargo test --workspace 2>&1 | grep -E "test result|FAILED|error\["
   ```
4. Record discoveries in `LEARNINGS.md`.

---

## Crate Dependency Order

Build and fix crates in this order — each depends on the ones before it:

```
1. protocol   (no internal dependencies)
2. knowledge  (no internal dependencies)
3. cortex     (depends on protocol, knowledge)
4. daemon     (depends on protocol, knowledge, cortex)
5. client     (depends on protocol)
```

When a compilation error occurs in `daemon`, check `protocol`, `knowledge`, and `cortex` first.

---

## Task Tracking Rules

- Before starting: add `## In Progress: TASK-NNN` to `LEARNINGS.md`.
- After completing: update to `## Completed: TASK-NNN`.
- After any bug: write full learning entry.

---

## Code Standards

### Rust Style
- Run `cargo clippy --workspace -- -D warnings` — must pass with 0 warnings
- Use `anyhow::Result` for error propagation in non-library public APIs
- Use `thiserror` for defining custom error types in library crates
- All public structs must derive `Debug`, `Clone`, `Serialize`, `Deserialize` where applicable
- Prefer `Arc<T>` + `RwLock<T>` over `Mutex<T>` for read-heavy shared state
- Use `tokio::spawn()` for background tasks; never `std::thread::spawn()` in async context

### Trait Implementations
- `Display` on all enum types that appear in user-facing output
- `From<T>` for error conversions rather than manual `.map_err()`

### Testing
- All unit tests in `#[cfg(test)] mod tests { ... }` at bottom of each `src/*.rs` file
- Integration tests in `crates/<crate>/tests/*.rs`
- Use `tempfile::TempDir` for filesystem isolation in tests
- Use `tokio::test` attribute for async tests
- Test both happy path AND error paths (file not found, permission denied, etc.)

### Serde Rules
- All message types in `organism-protocol` must roundtrip: `serialize → deserialize → equal`
- Use `#[serde(rename_all = "snake_case")]` on enums that appear in JSON
- Use `#[serde(tag = "kind")]` for event union types

---

## Common Mistakes — Read Before Coding

1. **Workspace vs crate Cargo.toml**: Add shared dependencies to workspace `Cargo.toml` under `[workspace.dependencies]`. Reference them in crate `Cargo.toml` as `dep = { workspace = true }`. Do NOT duplicate version numbers.

2. **tokio feature flags**: `tokio = { features = ["full"] }` is required for `#[tokio::main]` and `#[tokio::test]`. Check `Cargo.toml` includes `"full"`.

3. **broadcast::Receiver is not Clone**: `broadcast::Receiver<T>` cannot be cloned. Each subscriber must call `bus.subscribe()` to get their own receiver. Store the sender in `Arc`, not the receiver.

4. **`?` operator in main**: `fn main() -> anyhow::Result<()>` allows `?` in main. Without the return type, `?` causes a compile error.

5. **serde with chrono**: `chrono` timestamps require `features = ["serde"]` in `Cargo.toml`. Add: `chrono = { version = "0.4", features = ["serde"] }`.

6. **Dead code warnings become errors**: With `cargo clippy -- -D warnings`, `#[allow(dead_code)]` may be needed on stub implementations. Prefer removing unused code over suppressing warnings.

7. **RocksDB not yet added**: The current implementation uses a file-backed store (no native deps). Do NOT add `rocksdb` crate — it requires C++ compilation and will fail in many environments. The file-based store is the correct approach for now.

8. **Test isolation**: Every test that uses `KnowledgeStore` must use `tempfile::TempDir` — never `~/.organism/`. A dropped `TempDir` automatically deletes the temp directory.

9. **tokio::test vs test**: `#[test]` for sync tests, `#[tokio::test]` for async tests. Mixing them causes runtime panics.

10. **Cargo.lock**: Commit `Cargo.lock` for binaries (daemon, client), not for libraries. In a workspace with both, commit it — it's needed for reproducible builds.

---

## Learnings System

```markdown
## Learning NNN — YYYY-MM-DD: TASK-NNN — <title>
**Problem:** ...
**Root cause:** ...
**Fix:** ...
**Prevention:** ...
```

---

## Definition of Done

- [ ] All 10 tasks verified (exit 0)
- [ ] `cargo test --workspace` shows 0 failed
- [ ] `cargo clippy --workspace -- -D warnings` shows 0 warnings
- [ ] `cargo build --workspace --release` succeeds
- [ ] `./target/release/organism-cli help` prints usage
- [ ] `LEARNINGS.md` populated with discoveries

## What Comes Next (Level 1, not in this TASKS.md)

- Unix socket IPC between client and daemon
- zsh `preexec`/`precmd` hook shell script
- Real file watcher using `notify` crate
- Error signature classifier
- Ollama API client for local LLM inference
