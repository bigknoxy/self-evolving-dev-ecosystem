# 🎯 EXECUTION-PLAN.md — Sub-Agent Orchestration

> Master orchestration for executing TASKS.md (TASK-001 → TASK-010) via parallel sub-agents.
> This document is **the dispatcher's bible**. Each section gives a sub-agent everything it needs, no extra reading required.
>
> **Source of truth for task content:** `TASKS.md`
> **Source of truth for code standards:** `AGENTS.md` + `CLAUDE.md`
> **Source of truth for architecture:** `PLAN.md` + `IMPLEMENTATION.md`
> **This file:** _how_ to execute, in what order, by whom, with what gates.

---

## 0. Pre-Flight Checklist (Orchestrator only — run ONCE)

```bash
cd ~/projects/self-evolving-dev-ecosystem

# Toolchain
cargo --version || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install stable
rustup component add clippy rustfmt

# Working dir clean
ls AGENTS.md CLAUDE.md PLAN.md IMPLEMENTATION.md TASKS.md LEARNINGS.md  # all must exist
test ! -d crates && echo "fresh start" || echo "WARNING: crates/ already exists — review state first"
```

**Stop conditions:** `cargo --version` fails → user must install Rust manually. `crates/` already populated → orchestrator must read existing state before dispatching.

---

## 1. Dependency DAG

```
                  ┌──────────────┐
                  │  TASK-001    │  workspace skeleton (BLOCKS ALL)
                  └──────┬───────┘
                         │
          ┌──────────────┼──────────────┐
          ▼              ▼              ▼
    ┌─────────┐   ┌──────────┐   ┌───────────┐
    │TASK-002 │   │ TASK-004 │   │ TASK-007  │  ← only needs protocol stub
    │protocol │   │knowledge │   │  client   │     (can start after 002)
    │ types   │   │   store  │   └───────────┘
    └────┬────┘   └────┬─────┘
         ▼             │
    ┌─────────┐        │
    │TASK-003 │        │
    │protocol │        │
    │  tests  │        │
    └────┬────┘        │
         └──────┬──────┘
                ▼
          ┌──────────┐
          │ TASK-005 │  cortex (needs protocol + knowledge)
          └────┬─────┘
               ▼
          ┌──────────┐
          │ TASK-006 │  daemon (needs all above)
          └────┬─────┘
               ▼
          ┌──────────┐
          │ TASK-008 │  integration tests
          └────┬─────┘
               ▼
          ┌──────────┐
          │ TASK-009 │  CI + clippy gate
          └────┬─────┘
               ▼
          ┌──────────┐
          │ TASK-010 │  README + final verify
          └──────────┘
```

---

## 2. Parallel Execution Waves

Dispatch sub-agents in waves. **Each wave must fully verify before next wave begins.**

| Wave | Tasks (parallel) | Gate Command | Pass Criteria |
|------|------------------|--------------|---------------|
| **W0** | TASK-001 | `cargo build` | exit 0, both bins compile |
| **W1** | TASK-002 ∥ TASK-004 ∥ TASK-007-stub | `cargo build --workspace` | exit 0 |
| **W2** | TASK-003 ∥ TASK-005 | `cargo test -p organism-protocol -p organism-cortex -p organism-knowledge` | all `ok` |
| **W3** | TASK-006 | `cargo build -p organism-daemon` | exit 0 |
| **W4** | TASK-007-final ∥ TASK-008 | `cargo test -p organism-daemon -p organism-client` | all `ok` |
| **W5** | TASK-009 | `cargo clippy --workspace -- -D warnings` | 0 warnings |
| **W6** | TASK-010 | `cargo test --workspace && cargo build --workspace --release && ./target/release/organism-cli help` | all 3 succeed |

**TASK-007 split note:** stub in W1 (just `fn main()`), full impl in W4. This unblocks early parallelism.

---

## 3. Sub-Agent Dispatch Template

Use this exact prompt shape per task. Fill `{{TASK-NNN}}` and `{{wave}}`.

```
Task: Execute {{TASK-NNN}} from TASKS.md in /Users/Joshua.Knox/projects/self-evolving-dev-ecosystem.

Context:
- Read CLAUDE.md (allowed commands, code standards) before any tool call.
- Read AGENTS.md "Common Mistakes" section (10 items) — these prevent re-work.
- Read TASKS.md and locate {{TASK-NNN}}. Execute every step in order, exact code as written.
- This is wave {{wave}} of EXECUTION-PLAN.md. Prior waves verified passing. Do not modify files outside {{TASK-NNN}}'s "Files created" list.

Constraints:
- Use Write tool for new files, Edit tool for existing files. Never echo > file (loses formatting).
- Do not add dependencies beyond what TASKS.md specifies for this task.
- Do not skip the Verification step. If it fails, debug and re-run before reporting done.
- Use anyhow::Result, thiserror, Arc<RwLock<T>>, tokio::spawn — never std::thread::spawn or unwrap() outside tests.
- Tests use tempfile::TempDir, never ~/.organism.

Deliverable: Report in this exact format:
  STATUS: pass | fail
  FILES: <comma-separated paths created/modified>
  VERIFICATION: <last 5 lines of verification command output>
  LEARNINGS: <one line summary, or "none">
  IF FAIL: <root cause + remediation attempted>

Do NOT mark complete unless verification command exits 0 AND test result lines (if any) show "ok".
```

**For parallel waves:** dispatch all tasks in the wave in a single message with multiple Agent tool blocks. Do NOT serialize.

---

## 4. Per-Task Quick Reference

Below: dependencies, files touched, verification gate, common pitfalls. Full instructions in `TASKS.md`.

### TASK-001 — Workspace skeleton
- **Deps:** none
- **Files:** `Cargo.toml`, `crates/{protocol,knowledge,cortex,daemon,client}/Cargo.toml`, `crates/*/src/{lib.rs,main.rs}` stubs, `tests/integration/`
- **Gate:** `cargo build` exit 0
- **Pitfall:** Workspace deps with `version = "..."` duplicated in crate `Cargo.toml` — don't.

### TASK-002 — Protocol types
- **Deps:** 001
- **Files:** `crates/protocol/src/{lib.rs,events.rs,messages.rs}`
- **Gate:** `cargo test -p organism-protocol` exit 0
- **Pitfall:** chrono needs `features = ["serde"]` (already in workspace deps from 001).

### TASK-003 — Protocol tests
- **Deps:** 002
- **Files:** appends `#[cfg(test)] mod tests` block to `crates/protocol/src/lib.rs`
- **Gate:** `cargo test -p organism-protocol` shows `5 passed`
- **Pitfall:** `MessageType` uses `PascalCase` rename — assertions must match.

### TASK-004 — Knowledge store
- **Deps:** 001 (NOT 002 — independent)
- **Files:** `crates/knowledge/src/{lib.rs,store.rs,types.rs}`
- **Gate:** `cargo test -p organism-knowledge` shows `5 passed`
- **Pitfall:** key encoding `:` ↔ `_` is lossy — see G-007. Don't use `_` in subcomponents.

### TASK-005 — Cortex
- **Deps:** 002, 004
- **Files:** `crates/cortex/src/{lib.rs,context_detector.rs,pattern_engine.rs}`
- **Gate:** `cargo test -p organism-cortex` shows `5 passed`
- **Pitfall:** `detect_patterns` uses `windows(2)` — sequence of `[A,A,A]` yields pair `(A,A)` count 2.

### TASK-006 — Daemon skeleton
- **Deps:** 002, 004, 005
- **Files:** `crates/daemon/src/{main.rs,event_bus.rs,daemon.rs}`
- **Gate:** `cargo build -p organism-daemon` exit 0
- **Pitfall:** `broadcast::Receiver` not Clone (G-002). Store `Arc<EventBus>`, call `subscribe()` per task.

### TASK-007 — CLI client
- **Deps:** 002 (stub form), 006 (final form)
- **Files:** `crates/client/src/main.rs`
- **Gate:** `cargo run -p organism-client -- help` exits 0 with usage output
- **Pitfall:** `match` arm order — `_` catch-all must be last. Source has `"--help" | "help" | _` which compiles but unreachable warning possible; if clippy complains, separate `_` arm.

### TASK-008 — Integration tests
- **Deps:** 003, 004, 005, 006
- **Files:** `crates/daemon/tests/integration_test.rs`, append `[dev-dependencies]` to `crates/daemon/Cargo.toml`
- **Gate:** `cargo test -p organism-daemon` all pass
- **Pitfall:** integration test re-declares `EventBus` locally (avoiding lib/bin split). Keep this duplication — refactor is Level-1 work.

### TASK-009 — CI + clippy gate
- **Deps:** 008
- **Files:** `.github/workflows/ci.yml`, `.gitignore`
- **Gate:** `cargo clippy --workspace -- -D warnings` shows 0 warnings, full test suite green
- **Pitfall:** `~/.organism/` line in `.gitignore` won't match (no expansion) — change to `.organism/` or remove.

### TASK-010 — README + final verify
- **Deps:** 009
- **Files:** `README.md`
- **Gate:** all 3 commands pass: `cargo test --workspace`, `cargo build --workspace --release`, `./target/release/organism-cli help`
- **Pitfall:** none — pure docs + smoke.

---

## 5. Verification Cadence

After every wave AND every 3 tasks completed, the orchestrator runs:

```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test --workspace 2>&1 | grep -E "test result|FAILED|^error"
```

**Pass:** all `test result:` lines say `ok`, no `FAILED`, no `error[`.
**Fail:** orchestrator pauses, dispatches a debug sub-agent with the failing crate name and last 50 lines of output before continuing.

---

## 6. Failure Recovery Protocol

When a sub-agent reports `STATUS: fail`:

1. **Read its remediation attempt.** Do not blindly re-dispatch — that wastes a turn.
2. **Classify the failure:**
   - **Compilation error in own crate:** dispatch fix-agent with `cargo check -p <crate> 2>&1 | tail -50` output appended.
   - **Compilation error in dependent crate:** earlier wave was buggy. Re-verify W{n-1} before retrying current.
   - **Test assertion failure:** test logic likely wrong. Read test + impl, fix one, re-run.
   - **Clippy warning:** apply fix from AGENTS.md "Code Standards" section. Never `#[allow]` without justification (G-005 says prefer option b: add a consumer).
3. **Append to LEARNINGS.md** if root cause took >5 min to diagnose. Use template in AGENTS.md §"Learnings System".
4. **Re-run verification gate** before proceeding.

---

## 7. Final Acceptance (Definition of Done)

All boxes must be checked, in order, by the orchestrator after W6:

- [ ] `cargo test --workspace` → 0 failed, all suites `ok`
- [ ] `cargo clippy --workspace -- -D warnings` → 0 warnings
- [ ] `cargo build --workspace --release` → exit 0
- [ ] `./target/release/organism-cli help` → prints usage, exit 0
- [ ] `./target/release/organism &` then `kill %1` → starts and stops cleanly
- [ ] `LEARNINGS.md` has at least 10 completion entries (one per task)
- [ ] `git status` (if git initialized) → only expected new files

---

## 8. Out-of-Scope (Level 1+, do NOT attempt here)

These will fail or distract sub-agents. **Reject any sub-agent proposal to add them.**

- `rocksdb` crate (native deps — file store is correct for now)
- Unix socket IPC between daemon and client
- `notify` crate file watcher
- Ollama / LLM client
- zsh `preexec` / `precmd` hook scripts
- macOS LaunchAgent plist
- Encryption / SSH-key-derived DB encryption
- Plugin subprocess loader
- Web dashboard (Actix-web)
- Telemetry / Prometheus

These appear in PLAN.md and IMPLEMENTATION.md as future work — they are **read-only context**, not implementation targets for TASK-001 → TASK-010.

---

## 9. Orchestrator Quick-Start

```
1. Run §0 pre-flight.
2. Dispatch W0 (TASK-001) using §3 template.
3. On pass: dispatch W1 — three Agent calls in ONE message (002, 004, 007-stub).
4. On pass: dispatch W2 — two Agent calls in ONE message (003, 005).
5. On pass: dispatch W3 (006).
6. On pass: dispatch W4 — two Agent calls in ONE message (007-final, 008).
7. On pass: dispatch W5 (009).
8. On pass: dispatch W6 (010).
9. Run §7 final acceptance. Report to user.
```

End of EXECUTION-PLAN.md.
