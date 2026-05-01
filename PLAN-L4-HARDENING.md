# Plan: L4 Hardening — Close the Loop, Make It Daily-Usable

**Goal.** After this plan, you (the user) can install on a fresh box, restart, walk away for a week, and the daemon will quietly catch errors, dedupe LLM calls, log to disk, list hashes for you, and accept/reject feedback that becomes L4 training signal.

**Non-goals.** L4 model training itself. Web UI. Remote sync. Windows.

**Style.** Each task: ≤300 LoC, ≤6 files touched, has a failing-test-first acceptance gate. Local model (qwen2.5-coder:7b via `mcp__ollama-dev__implement_task`) implements; you (or main agent) review the diff against the gate. If gate fails, reject and retry with the failing test as the prompt anchor.

**Branch strategy.** One branch per task family (M1-M9). Each task is a commit on that branch. PR per family. CI must be green before merge. Tag `v0.4.0` after M1-M5 (daily-usable), `v0.5.0` after M6-M9 (L4-ready).

---

## Dependency DAG

```
M1 (errors CLI) ──┐
M2 (dedupe)      ─┼─> M5 (plist env) ──> v0.4.0 ──> M6 (feedback) ──┐
M3 (logging)    ──┤                                                  ├─> M9 (schema_v) ──> v0.5.0
M4 (shutdown)    ─┘                                  M7 (multi-block)─┤
                                                     M8 (PII guard) ──┘
```

M1-M4 are independent — implement in parallel branches.

---

# M1 — `organism-cli errors` listing command

**Why.** Today, user must `ls $ORGANISM_HOME/knowledge/error_*.json | sed ...` to find hashes for `apply`. Removes #1 daily-use friction.

## TASK-M1-01 — `KnowledgeStore::list_errors`

- **File**: `crates/knowledge/src/store.rs` (modify)
- **Add**:
  ```rust
  pub struct ErrorSummary {
      pub hash: String,
      pub last_command: String,
      pub occurrences: u32,
      pub last_seen: chrono::DateTime<chrono::Utc>,
      pub has_suggestion: bool,
  }
  pub fn list_errors(&self, limit: usize) -> Result<Vec<ErrorSummary>>;
  ```
- **Behavior**: scan `error_*.json` in knowledge dir, parse, sort by `last_seen DESC`, take `limit`, fill `has_suggestion` by checking `suggestion_<hash>.json` existence.
- **Tests** (add to `#[cfg(test)] mod tests`):
  1. empty store → empty vec
  2. seed 3 errors w/ different `last_seen` → returns sorted DESC
  3. `limit = 2` truncates
  4. `has_suggestion` true when paired file exists, false otherwise
  5. corrupt `error_zzz.json` skipped, not returned, no panic
- **Acceptance**: `cargo test -p organism-knowledge list_errors` → 5 passed.

## TASK-M1-02 — Protocol `errors` request

- **File**: `crates/protocol/src/messages.rs`
- **Add**:
  ```rust
  pub struct ErrorsRequest { pub limit: Option<usize> }
  pub struct ErrorsResponse { pub items: Vec<ErrorSummaryWire> }
  pub struct ErrorSummaryWire {
      pub hash: String,
      pub command: String,
      pub occurrences: u32,
      pub last_seen: String,  // RFC3339
      pub has_suggestion: bool,
  }
  ```
- Re-export in `lib.rs`.
- **Tests**: 2 roundtrip tests (empty `items`, 3-item).
- **Acceptance**: `cargo test -p organism-protocol errors_` → 2 passed.

## TASK-M1-03 — Daemon `errors` IPC handler

- **File**: `crates/daemon/src/ipc.rs`
- Add `"errors" => { ... }` arm. Calls `knowledge.list_errors(req.limit.unwrap_or(20))`, maps to `ErrorSummaryWire` (RFC3339 via `chrono::SecondsFormat::Secs`).
- **Tests**: integration test `crates/daemon/tests/errors_test.rs`. Seed 2 errors via `KnowledgeStore::put_error`, call IPC, assert 2 items, sorted, RFC3339 parses.
- **Acceptance**: `cargo test -p organism-daemon --test errors_test` → green.

## TASK-M1-04 — CLI `errors` command

- **File**: `crates/client/src/main.rs`
- Add `"errors" => cmd_errors(&args[2..]).await` dispatch.
- Flag: `--limit N` (default 20), `--json` (raw output for piping).
- Default human format:
  ```
  HASH      AGE     OCC  SUG  COMMAND
  deadbeef  3m ago  4    yes  cargo build --workspace
  cafef00d  1h ago  1    no   pnpm test
  ```
- Use `chrono` to render age (≤60s "Ns", ≤60m "Nm", ≤24h "Nh", else "Nd").
- **Tests**: 4 unit tests on `format_age()` (boundaries: 59s, 61s, 59m, 25h); 1 integration test that boots daemon, seeds 2 errors, runs `cmd_errors`, asserts both hashes appear in stdout (capture via `print!` indirection or test-only return).
- **Acceptance**: `cargo test -p organism-client errors` → 5 passed.

## M1 Verification (manual, post-merge)

```bash
organism-cli emit-terminal "cargo build" --exit-code 1 --stderr "error[E0599]"
organism-cli errors                # should list 1 row
organism-cli errors --json | jq .  # valid JSON
```

---

# M2 — Suggestion dedupe + cache-aware apply

**Why.** Currently every red `cargo build` triggers an LLM call. Repeat-failure spam. Also `apply` re-fetches even when cached.

## TASK-M2-01 — Cache check in daemon Ollama subscriber

- **File**: `crates/daemon/src/main.rs` (or wherever subscriber lives)
- **Change**: before calling `suggest_for_error`, check `knowledge.get_suggestion(hash)?.is_some()`. If yes, skip. Log `tracing::debug!("suggestion cached for {}", hash)`.
- **Tests**: refactor subscriber loop into `pub async fn handle_event(...)` so it's unit-testable. Test 1: cached → returns `SkippedCached`. Test 2: not cached → calls mock LlmClient. Use enum return:
  ```rust
  pub enum SubscriberOutcome { SkippedCached, Generated, Skipped(Reason) }
  ```
- **Acceptance**: `cargo test -p organism-daemon subscriber_dedupe` → 2 passed.

## TASK-M2-02 — Time-window dedupe (`occurrences` bump only)

- **File**: `crates/cortex/src/classifier.rs` (or wherever errors are recorded)
- **Behavior**: if same hash seen <60s ago, bump `occurrences`, do NOT broadcast a new "generate suggestion" event. (Bus already broadcasts; gate the broadcast.)
- Add field to bus event: `is_first_in_window: bool`. Subscriber only acts when true.
- **Tests**: 3 tests. (a) first occurrence → `is_first_in_window=true`. (b) second within 60s → `false`. (c) second after 61s → `true`. Use `tokio::time::pause()` + `advance()`.
- **Acceptance**: `cargo test -p organism-cortex window_dedupe` → 3 passed.

## TASK-M2-03 — `suggest --regenerate` flag

- **File**: `crates/client/src/main.rs`, `crates/protocol/src/messages.rs`
- Add `force: bool` to `SuggestRequest`. CLI `suggest --regenerate` sets it.
- Daemon: when `force=true`, delete cached suggestion before generating.
- **Tests**: 1 protocol roundtrip, 1 daemon integration (seed cached, force regen, assert new content).
- **Acceptance**: `cargo test --workspace regenerate` → 2 passed.

## M2 Verification (manual)

```bash
# Trigger same failure 5x in 10s
for i in 1 2 3 4 5; do
  organism-cli emit-terminal "cargo build" --exit-code 1 --stderr "error[E0599]: same"
done
sleep 2
ls $ORGANISM_HOME/knowledge/suggestion_*.json | wc -l   # should be 1, not 5
```

Counter assertion: tail daemon log, expect 4 `suggestion cached` lines.

---

# M3 — File logging via `tracing-appender`

**Why.** Daemon logs to in-memory ring buffer only. Crash = no trace. Can't debug user reports.

## TASK-M3-01 — Add `tracing-subscriber` + `tracing-appender` deps

- **File**: `Cargo.toml` (workspace), `crates/daemon/Cargo.toml`
- Add to workspace deps: `tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }`, `tracing-appender = "0.2"`.
- **Tests**: `cargo build -p organism-daemon` succeeds. (No test; this is a deps-only commit.)
- **Acceptance**: workspace builds.

## TASK-M3-02 — Init file logger in `main.rs`

- **File**: `crates/daemon/src/main.rs`
- On startup:
  ```rust
  let log_dir = organism_home().join("logs");
  std::fs::create_dir_all(&log_dir)?;
  let file_appender = tracing_appender::rolling::daily(&log_dir, "daemon.log");
  let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
  tracing_subscriber::fmt()
      .with_writer(non_blocking)
      .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
      .json()
      .init();
  // CRITICAL: hold _guard for daemon lifetime
  ```
- Bind `_guard` to `let _log_guard = ...;` in `main()` scope. **Document** in comment: dropping the guard truncates pending writes.
- **Tests**: integration test `tests/logging_test.rs`: spawn daemon w/ tempdir as `ORGANISM_HOME`, send IPC `status`, assert `<tempdir>/logs/daemon.log.YYYY-MM-DD` exists and contains `"status"`.
- **Acceptance**: `cargo test -p organism-daemon --test logging_test` green.

## TASK-M3-03 — Log rotation policy

- **File**: same `main.rs`
- Use `tracing_appender::rolling::Builder` to keep last 7 daily files (`max_log_files(7)`).
- **Tests**: skip (rotation is `tracing-appender` internal). Document in `LEARNINGS.md`.
- **Acceptance**: code review only.

## M3 Verification (manual)

```bash
ORGANISM_HOME=/tmp/orgsmoke organism-daemon &
sleep 2; organism-cli status; sleep 1
cat /tmp/orgsmoke/logs/daemon.log.* | head -5   # JSON lines, "status" present
```

---

# M4 — Graceful shutdown on SIGTERM/SIGINT

**Why.** `tokio::spawn` tasks abort mid-write. Corrupt JSON in `~/.organism/knowledge/` is unrecoverable.

## TASK-M4-01 — Shutdown signal channel

- **File**: `crates/daemon/src/main.rs`
- Add:
  ```rust
  use tokio::signal::unix::{signal, SignalKind};
  let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
  let mut sigterm = signal(SignalKind::terminate())?;
  let mut sigint  = signal(SignalKind::interrupt())?;
  let stx = shutdown_tx.clone();
  tokio::spawn(async move {
      tokio::select! { _ = sigterm.recv() => {}, _ = sigint.recv() => {} }
      let _ = stx.send(());
  });
  ```
- Pass `shutdown_tx.subscribe()` to each long-lived task (ipc::serve, file_watcher, ollama subscriber).
- **Tests**: unit test for the signal-fan-out helper (refactor into `pub fn build_shutdown() -> (Sender, ...)`); test that `send()` is observed by N subscribers.
- **Acceptance**: `cargo test -p organism-daemon shutdown_signal` green.

## TASK-M4-02 — `tokio::select!` in each subscriber

- **Files**: `crates/daemon/src/ipc.rs`, `crates/daemon/src/main.rs` (file watcher loop, Ollama subscriber loop)
- Each `loop { ... }` becomes:
  ```rust
  loop {
      tokio::select! {
          _ = shutdown.recv() => break,
          msg = bus_rx.recv() => { handle(msg).await; }
      }
  }
  ```
- IPC server: between `accept()` calls, select on shutdown.
- **Tests**: integration test `tests/shutdown_test.rs`. Spawn full daemon, send `shutdown_tx.send(())` (expose it via test-only entry point), assert all task handles complete within 2s. Use `tokio::time::timeout`.
- **Acceptance**: `cargo test -p organism-daemon --test shutdown_test` green, completes <3s.

## TASK-M4-03 — Atomic JSON writes in KnowledgeStore

- **File**: `crates/knowledge/src/store.rs`
- Replace direct `std::fs::write(path, json)` with write-temp-then-rename:
  ```rust
  fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
      let tmp = path.with_extension("tmp");
      std::fs::write(&tmp, bytes)?;
      std::fs::rename(&tmp, path)?;
      Ok(())
  }
  ```
- Apply to `put_error`, `put_pattern`, `put_suggestion`.
- **Tests**: 1 test that interrupts mid-write (write huge value, kill thread mid-flight is hard; instead test the helper directly: write, assert no `.tmp` left). 1 test verifying serialized 1MB suggestion succeeds.
- **Acceptance**: `cargo test -p organism-knowledge atomic_write` → 2 passed.

## M4 Verification (manual)

```bash
organism-daemon &
PID=$!
# Trigger writes
for i in 1..10; do organism-cli emit-terminal "cmd $i" --exit-code 1 --stderr "err"; done
kill -TERM $PID
wait $PID
echo "exit=$?"   # should be 0, not 137
ls $ORGANISM_HOME/knowledge/*.tmp 2>/dev/null    # nothing
```

---

# M5 — LaunchAgent `EnvironmentVariables`

**Why.** Plist daemon doesn't see `OLLAMA_ENABLED=1` from your shell. Loop never closes on real daily use.

## TASK-M5-01 — Plist template with EnvironmentVariables

- **File**: `scripts/install.sh` (or wherever plist is generated)
- Find current plist heredoc. Add:
  ```xml
  <key>EnvironmentVariables</key>
  <dict>
      <key>OLLAMA_ENABLED</key><string>1</string>
      <key>OLLAMA_BASE_URL</key><string>http://127.0.0.1:11434</string>
      <key>OLLAMA_MODEL</key><string>qwen2.5-coder:7b</string>
      <key>ORGANISM_HOME</key><string>__HOME__/.organism</string>
      <key>PATH</key><string>__HOME__/.local/bin:/usr/local/bin:/usr/bin:/bin</string>
      <key>RUST_LOG</key><string>info</string>
  </dict>
  ```
- Substitute `__HOME__` with `$HOME` in the install script (`sed -i.bak "s|__HOME__|$HOME|g"`).
- **Tests**: shell test in `scripts/test-install.sh`. Run install with `HOME=$tmp`, then `plutil -lint $tmp/Library/LaunchAgents/com.organism.daemon.plist` → `OK`. Then `grep -c "OLLAMA_ENABLED" $plist` → `1`.
- **Acceptance**: `bash scripts/test-install.sh` exit 0.

## TASK-M5-02 — Allow override via `~/.organism/env`

- **File**: `scripts/install.sh`
- If `~/.organism/env` exists, append its `KEY=VAL` lines as additional `<key><string>` pairs in the dict.
- Document in README: "Override defaults by writing `KEY=VAL` lines to `~/.organism/env` before install".
- **Tests**: extend `test-install.sh`: write `~/.organism/env` with `OLLAMA_MODEL=llama3:8b`, install, assert plist contains `llama3:8b`.
- **Acceptance**: shell test passes.

## M5 Verification (manual, on this Mac)

```bash
echo 'OLLAMA_MODEL=qwen2.5-coder:7b' > ~/.organism/env
bash scripts/install.sh
launchctl unload ~/Library/LaunchAgents/com.organism.daemon.plist
launchctl load   ~/Library/LaunchAgents/com.organism.daemon.plist
sleep 2
launchctl list | grep organism                    # PID present
ps Eww $(pgrep organism-daemon) | grep OLLAMA     # env vars present
```

---

## TAG v0.4.0 — daily-usable

After M1-M5 merged: `git tag -a v0.4.0 -m "v0.4.0 — daily-usable: errors CLI, dedupe, file logs, graceful shutdown, plist env"`. Push tag. Watch release.yml. Verify artifacts.

**Dogfood gate (mandatory before M6):** install v0.4.0 on real `~/`, leave running 7 days, accumulate ≥20 real errors, run `organism-cli errors`, hand-review suggestions. If ≥40% are useful, proceed. Else reopen M-series with prompt-engineering tasks (out of scope here).

---

# M6 — Accept/reject feedback capture (L4 training signal)

**Why.** Knowledge store grows but never gets better. L4 model needs labels.

## TASK-M6-01 — Feedback record type

- **File**: `crates/knowledge/src/types.rs`, `crates/knowledge/src/store.rs`
- Add:
  ```rust
  pub struct FeedbackRecord {
      pub error_hash: String,
      pub suggestion_hash: String,    // sha256 of suggestion text
      pub verdict: Verdict,            // Accepted, Rejected, Ignored
      pub note: Option<String>,
      pub ts: chrono::DateTime<chrono::Utc>,
  }
  pub enum Verdict { Accepted, Rejected, Ignored }
  pub fn put_feedback(&mut self, fb: &FeedbackRecord) -> Result<()>;
  pub fn list_feedback(&self) -> Result<Vec<FeedbackRecord>>;
  ```
- File: `feedback_<error_hash>_<ts_unix>.json` (timestamped to allow multiple per error).
- **Tests**: 4 (put+get roundtrip, list, multiple per hash, corrupt file skipped).
- **Acceptance**: `cargo test -p organism-knowledge feedback` → 4 passed.

## TASK-M6-02 — Protocol `feedback` request

- **File**: `crates/protocol/src/messages.rs`
- `FeedbackRequest { error_key, verdict: "accept"|"reject"|"ignore", note: Option<String> }`
- `FeedbackResponse { ok: bool }`
- **Tests**: 3 roundtrip (each verdict).
- **Acceptance**: green.

## TASK-M6-03 — Daemon handler + CLI command

- Files: `crates/daemon/src/ipc.rs`, `crates/client/src/main.rs`
- CLI: `organism-cli feedback <hash> accept|reject [--note "..."]`
- Daemon writes FeedbackRecord. Hash suggestion text via `sha2::Sha256` (already in deps for hash gen).
- **Tests**: 1 daemon integration (seed error+suggestion, send feedback, assert file present, JSON parses).
- **Acceptance**: green.

## TASK-M6-04 — Auto-feedback on `apply --stage`

- File: `crates/daemon/src/ipc.rs`
- When `apply --stage` succeeds (artifact written or clipboard set), record implicit `Verdict::Accepted` feedback automatically.
- Rationale: user staging is strongest signal short of running it.
- **Tests**: 1 integration. Apply --stage; immediately call `list_feedback`; assert 1 record with verdict=Accepted.
- **Acceptance**: green.

## M6 Verification (manual)

```bash
organism-cli apply <hash> --stage
ls $ORGANISM_HOME/knowledge/feedback_*.json    # ≥1
organism-cli feedback <other_hash> reject --note "wrong file"
cat $ORGANISM_HOME/knowledge/feedback_*.json | jq .verdict
```

---

# M7 — Multi-block plan parsing

**Why.** Suggestions like "first run X, then apply Y" collapse to whichever fence appears first. Wrong for shell-then-patch flows.

## TASK-M7-01 — `extract_plans` returns `Vec<ApplyPlan>`

- File: `crates/cortex/src/apply.rs`
- Keep existing `extract_plan` for backward compat; add:
  ```rust
  pub fn extract_plans(suggestion: &str) -> Vec<ApplyPlan>;
  ```
- Returns ordered list of all recognized fences. Note-only suggestions return single-element `[Note { text }]`.
- **Tests**: 5. (a) one diff → `[Patch]`. (b) bash then diff → `[Shell, Patch]`. (c) three blocks → 3 items in order. (d) no fences → `[Note]`. (e) unknown lang fences ignored, recognized ones kept in order.
- **Acceptance**: `cargo test -p organism-cortex extract_plans` → 5 passed.

## TASK-M7-02 — Protocol `ApplyResponse` carries `Vec<PlanItem>`

- File: `crates/protocol/src/messages.rs`
- Add `pub plans: Vec<PlanItemWire>` field. Keep existing `plan_kind`/`message`/`artifact_path` populated from `plans[0]` for backward compat.
- `PlanItemWire { kind, body, artifact_path: Option<String>, clipboard: bool }`.
- **Tests**: 2 (single, multi).
- **Acceptance**: green.

## TASK-M7-03 — Daemon stages all plans

- File: `crates/daemon/src/ipc.rs`
- For `--stage`: write each Patch as `/tmp/organism-<hash>-<idx>.patch`. Concat shell commands and copy joined block to clipboard (or, if multi, write `/tmp/organism-<hash>.sh` instead).
- **Tests**: integration. Seed suggestion with bash + diff. Apply --stage. Assert both `.sh` and `.patch` files present.
- **Acceptance**: green.

## TASK-M7-04 — CLI prints all plans numbered

- File: `crates/client/src/main.rs`
- Output:
  ```
  [1/2] shell
      brew install foo
  [2/2] patch
      diff --git a/x b/x
      ...
  ```
- **Tests**: 2 unit tests on output formatter.
- **Acceptance**: green.

## M7 Verification (manual)

Seed via `KnowledgeStore::put_suggestion("abc", "Run:\n```bash\nfoo\n```\nThen patch:\n```diff\n-x\n+y\n```\n")`, then `organism-cli apply abc` shows 2 numbered blocks.

---

# M8 — PII / leakage guard for Ollama prompts

**Why.** Stderr snippets sent to LLM. Local-only today, but if `OLLAMA_BASE_URL` ever points remote, file paths and secrets leak.

## TASK-M8-01 — `cortex::redact` module

- File: `crates/cortex/src/redact.rs` (new)
- ```rust
  pub fn redact(input: &str) -> String;
  ```
- Rules:
  1. Absolute home paths → `$HOME/...` (regex `/Users/[^/\s]+` or `/home/[^/\s]+`).
  2. Anything matching `(?i)(api[_-]?key|token|secret|password)\s*[:=]\s*\S+` → `<KEY>=<REDACTED>`.
  3. AWS-style keys `AKIA[0-9A-Z]{16}` → `<AWS_KEY_REDACTED>`.
  4. Bearer headers `Bearer\s+\S+` → `Bearer <REDACTED>`.
  5. Email addresses `[\w.+-]+@[\w-]+\.\w+` → `<EMAIL>`.
- Compile regexes once via `OnceLock` (CLAUDE.md rule).
- **Tests**: 8 — one per rule + one for combined input + one for "nothing to redact" passthrough.
- **Acceptance**: `cargo test -p organism-cortex redact` → 8 passed.

## TASK-M8-02 — Wire into `suggest_for_error`

- File: `crates/cortex/src/suggest.rs`
- Apply `redact()` to `command`, `stderr_snippet`, `cwd` before formatting prompt.
- **Tests**: existing suggest tests should still pass; add 1 that asserts redacted prompt is what hits the mock LlmClient.
- **Acceptance**: green.

## TASK-M8-03 — Remote-URL warning gate

- File: `crates/ollama/src/lib.rs` (or wherever client constructed)
- On startup, if `OLLAMA_BASE_URL` is not `localhost`/`127.0.0.1`/`::1`, emit `tracing::warn!("OLLAMA_BASE_URL is remote; redaction is best-effort, do not enable on shared hosts")`.
- Add env `OLLAMA_ALLOW_REMOTE=1` to suppress; otherwise refuse to start (`anyhow::bail!`).
- **Tests**: 3 — local URL OK, remote URL without flag errors, remote with flag warns but proceeds.
- **Acceptance**: `cargo test -p organism-ollama remote_url` → 3 passed.

## M8 Verification (manual)

```bash
OLLAMA_BASE_URL=http://example.com:11434 organism-daemon
# expect: error "remote OLLAMA_BASE_URL refused; set OLLAMA_ALLOW_REMOTE=1"
OLLAMA_BASE_URL=http://example.com:11434 OLLAMA_ALLOW_REMOTE=1 organism-daemon &
grep "is remote" $ORGANISM_HOME/logs/daemon.log.*
```

---

# M9 — Schema versioning

**Why.** `error_<hash>.json` has no `schema_v` field. Any change to `ErrorRecord` silently corrupts existing user state.

## TASK-M9-01 — Add `schema_v` field

- File: `crates/knowledge/src/types.rs`
- ```rust
  #[derive(Serialize, Deserialize, ...)]
  pub struct ErrorRecord {
      #[serde(default = "default_schema_v")]
      pub schema_v: u32,
      // ... existing fields
  }
  fn default_schema_v() -> u32 { 1 }
  ```
- Same for `PatternRecord`, `FeedbackRecord` (M6).
- **Tests**: 3 — fresh write has `schema_v=1`; old JSON without field deserializes to `schema_v=1`; explicit different value preserved.
- **Acceptance**: green.

## TASK-M9-02 — Migration shim

- File: `crates/knowledge/src/migrate.rs` (new)
- ```rust
  pub fn migrate_error(raw: serde_json::Value) -> Result<ErrorRecord>;
  ```
- For now: just calls `serde_json::from_value` since v1 is current. Stub for future v2→v3 paths. Document pattern.
- KnowledgeStore::get_error uses migrate_error instead of direct from_str.
- **Tests**: 2 — current v1 roundtrips; explicit `{"schema_v": 99, ...}` returns clean error mentioning version.
- **Acceptance**: green.

## TASK-M9-03 — `organism-cli doctor` self-check

- File: `crates/client/src/main.rs`
- New command: scans `$ORGANISM_HOME/knowledge/*.json`, reports:
  - count by type (error / pattern / suggestion / feedback)
  - any failing schema migrations
  - disk usage
  - daemon awake state via IPC `status`
- Exit 0 if healthy, 1 if any corruption.
- **Tests**: 3 — clean store → exit 0; seeded with one bad-version file → exit 1, error message identifies file.
- **Acceptance**: green.

## M9 Verification (manual)

```bash
organism-cli doctor
# Expected:
#   knowledge: 12 errors, 8 suggestions, 3 patterns, 5 feedback
#   schema:    all v1 ✓
#   disk:      2.3 MB
#   daemon:    awake ✓
#   OK
```

---

## TAG v0.5.0 — L4-ready

After M6-M9: tag `v0.5.0`. Verify release artifacts include darwin + linux. Update README Status matrix.

---

# Critical Files Index

| Milestone | File | Action |
|-----------|------|--------|
| M1 | `crates/knowledge/src/store.rs` | edit (`list_errors`) |
| M1 | `crates/protocol/src/messages.rs` | edit (`ErrorsRequest/Response`) |
| M1 | `crates/daemon/src/ipc.rs` | edit (handler) |
| M1 | `crates/daemon/tests/errors_test.rs` | new |
| M1 | `crates/client/src/main.rs` | edit (cmd_errors, format_age) |
| M2 | `crates/daemon/src/main.rs` | edit (subscriber dedupe) |
| M2 | `crates/cortex/src/classifier.rs` | edit (window dedupe) |
| M2 | `crates/protocol/src/messages.rs` | edit (`force` field) |
| M3 | `Cargo.toml`, `crates/daemon/Cargo.toml` | edit (deps) |
| M3 | `crates/daemon/src/main.rs` | edit (logger init) |
| M3 | `crates/daemon/tests/logging_test.rs` | new |
| M4 | `crates/daemon/src/main.rs` | edit (signal handler) |
| M4 | `crates/daemon/src/ipc.rs` | edit (select! shutdown) |
| M4 | `crates/knowledge/src/store.rs` | edit (atomic_write) |
| M4 | `crates/daemon/tests/shutdown_test.rs` | new |
| M5 | `scripts/install.sh` | edit (plist EnvironmentVariables) |
| M5 | `scripts/test-install.sh` | new |
| M6 | `crates/knowledge/src/types.rs` | edit (FeedbackRecord) |
| M6 | `crates/knowledge/src/store.rs` | edit (put/list_feedback) |
| M6 | `crates/protocol/src/messages.rs` | edit (FeedbackRequest) |
| M6 | `crates/daemon/src/ipc.rs` | edit (handler + auto-accept) |
| M6 | `crates/client/src/main.rs` | edit (cmd_feedback) |
| M7 | `crates/cortex/src/apply.rs` | edit (extract_plans) |
| M7 | `crates/protocol/src/messages.rs` | edit (PlanItemWire) |
| M7 | `crates/daemon/src/ipc.rs` | edit (multi-stage) |
| M7 | `crates/client/src/main.rs` | edit (numbered output) |
| M8 | `crates/cortex/src/redact.rs` | new |
| M8 | `crates/cortex/src/suggest.rs` | edit (wire redact) |
| M8 | `crates/ollama/src/lib.rs` | edit (remote URL gate) |
| M9 | `crates/knowledge/src/types.rs` | edit (schema_v) |
| M9 | `crates/knowledge/src/migrate.rs` | new |
| M9 | `crates/client/src/main.rs` | edit (cmd_doctor) |

---

# Acceptance Gate (apply per task)

A task is **DONE** only when:

1. New tests are written FIRST and fail before implementation.
2. `cargo test -p <crate> <test_filter>` shows expected count green.
3. `cargo clippy --workspace --all-targets -- -D warnings` clean.
4. `cargo fmt --all -- --check` clean.
5. Manual M-level verification command (above) reproduces expected output.
6. No `unwrap()` outside test code (CLAUDE.md).
7. No new file paths leak in error messages without redact().
8. Diff ≤ 300 LoC. If larger, split.

---

# Ollama Subagent Prompt Template

For each TASK above, prompt local model with this skeleton:

```
You are implementing TASK-<id> in a Rust workspace.

CONTEXT (read first):
- /Users/Joshua.Knox/projects/self-evolving-dev-ecosystem/CLAUDE.md
- <list of files this task touches>

GOAL: <copy "Behavior" / "Add" section verbatim>

TESTS (write these in same file, must compile and FAIL before code added):
<copy test list verbatim>

CONSTRAINTS:
- No `unwrap()` outside `#[cfg(test)]`. Use `OnceLock` + `.expect("literal")` for compile-time-known-good values.
- All public types: `#[derive(Debug, Clone, Serialize, Deserialize)]`.
- Error type: `anyhow::Result` in binaries, `thiserror` in libs.
- `#[tokio::test]` for async tests, `#[test]` for sync.

OUTPUT: full file contents for each modified file. No prose.
```

---

# Pitfalls Pre-Flagged

- **M3 `_log_guard` lifetime**: drop = silent log loss. Bind in `main()` not in helper that returns.
- **M4 broadcast vs mpsc**: shutdown signal must be `broadcast` so multiple subscribers each see it. Don't use `oneshot`.
- **M4 `notify` file watcher** doesn't natively support shutdown — wrap its event channel in `select!` instead.
- **M5 plist `__HOME__` substitution** — sed `-i.bak` differs between BSD (macOS) and GNU; use `sed -i.bak '...'` explicitly and `rm *.bak` after.
- **M6 suggestion hash** — use sha256 of *raw text* not of normalized text, so feedback ties to exactly what user saw.
- **M7 backward compat** — keep `plan_kind`/`message` populated from `plans[0]` so older CLI clients still work mid-rollout.
- **M8 redact email** — overzealous regex hits Cargo.toml `authors = "Foo <foo@bar.com>"`. Test against real Cargo.toml content.
- **M9 schema_v default** — `#[serde(default = ...)]` only fires when field MISSING. Existing files without the field auto-migrate. Adding it to a new field someday → fine.
- **General**: dogfood gate after M5 is real. If suggestion quality is <40% useful, M6's "feedback" data is garbage in / garbage out for L4. Don't skip.

---

# Verification Matrix (final, end-to-end)

| Gap (from CEO/EM review) | Closed by | Proof command |
|--------------------------|-----------|---------------|
| User has to `ls` for hashes | M1 | `organism-cli errors` shows recent rows |
| LLM call spam on repeats | M2 | 5 identical errors → 1 suggestion file |
| `--stage` re-fetches cached | M2 | second `apply` shows `(cached)` tag |
| No file logs / crash trace | M3 | `~/.organism/logs/daemon.log.*` exists |
| SIGTERM corrupts JSON | M4 | `kill -TERM` → exit 0, no `.tmp` files |
| Plist drops env vars | M5 | `ps Eww $(pgrep organism-daemon)` shows OLLAMA_ENABLED=1 |
| No accept/reject signal | M6 | `feedback_*.json` after `apply --stage` |
| Multi-block suggestions collapse | M7 | bash+diff suggestion → 2 staged artifacts |
| PII leaks to remote LLM | M8 | remote URL refused without explicit flag |
| Schema changes break users | M9 | `organism-cli doctor` reports schema versions |

---

# Next Action

Spawn 4 parallel branches: M1, M2, M3, M4. Each is independent. Use `mcp__ollama-dev__implement_task` per TASK with the prompt template above. Main agent reviews diffs against the acceptance gate before commit. M5 follows after M3+M4 merge (depends on log path + signal handling). Tag `v0.4.0`. Dogfood 7 days. Then M6-M9 sequentially. Tag `v0.5.0`.
