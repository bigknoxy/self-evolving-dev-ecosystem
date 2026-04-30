# Starting Prompt — Self-Evolving Dev Ecosystem (Organism)

> Paste this entire prompt into Claude Code (or your coding tool) to begin execution.

---

## Prompt

You are implementing the **self-evolving-dev-ecosystem** project — a Rust workspace daemon called "Organism" that learns developer patterns and acts as a persistent background assistant.

**Your working directory is:** `~/projects/self-evolving-dev-ecosystem`

**This project is Level 0 (Observer):** You are building the foundation — event bus, knowledge store, pattern engine, and CLI skeleton. Higher capability levels (IPC, file watcher, LLM integration) come after this foundation is solid.

**Read these files FIRST, in this exact order, before writing any code:**
1. `AGENTS.md` — Rust toolchain requirements, crate dependency order, common mistakes
2. `CLAUDE.md` — allowed commands, workspace structure, Rust code standards
3. `PLAN.md` — architecture, capability levels, live scenarios
4. `IMPLEMENTATION.md` — crate layout, IPC protocol, knowledge schema, plugin API
5. `TASKS.md` — the atomic task list you will execute

**Your mission:** Work through `TASKS.md` from TASK-001 to TASK-010, completing each task fully before moving to the next.

**Prerequisite check — run before TASK-001:**
```bash
cargo --version    # must succeed (>= 1.70.0)
rustup component add clippy
rustup component add rustfmt
```
If `cargo` is not found: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh && source "$HOME/.cargo/env"`

**Crate build order (CRITICAL — follow this):**
1. `protocol` (no deps) → TASK-002, TASK-003
2. `knowledge` (no deps) → TASK-004
3. `cortex` (deps: protocol, knowledge) → TASK-005
4. `daemon` (deps: all above) → TASK-006
5. `client` (deps: protocol) → TASK-007

**Task execution protocol (MANDATORY):**
1. Read the full task before writing any code
2. Execute every step in the exact order written
3. Run the `### Verification` command at the end of each task
4. If verification exits 0 → move to the next task
5. If verification fails → debug, fix, re-run, then proceed
6. NEVER skip a verification step
7. `cargo build` success is NOT sufficient — `cargo test -p <crate>` must also pass

**After completing each task:**
- Append a completion note to `LEARNINGS.md`
- If you hit a compiler/clippy error that took > 5 minutes, write a full learning entry

**After every 3 tasks, run the full workspace test suite:**
```bash
cd ~/projects/self-evolving-dev-ecosystem
cargo test --workspace 2>&1 | grep -E "test result|FAILED|error\["
```
All test results must show `ok`. No FAILED lines allowed.

**The project is complete when:**
- All 10 tasks verified ✓
- `cargo test --workspace` shows 0 failed
- `cargo clippy --workspace -- -D warnings` shows 0 warnings
- `cargo build --workspace --release` succeeds
- `./target/release/organism-cli help` prints usage without error
- `LEARNINGS.md` is populated

Begin by reading `AGENTS.md`, checking your Rust toolchain, then `TASKS.md`, then start TASK-001.
