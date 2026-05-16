# Organism — Implementation Details

> Engineering reference for the organism daemon. Covers crate layout, IPC protocol,
> knowledge store schema, LLM integration, and testing conventions.
> Updated through M17.

---

## 1. Repo Layout (Rust workspace)

```
self-evolving-dev-ecosystem/
├── Cargo.toml                  — workspace manifest (shared deps here)
├── crates/
│   ├── protocol/               — event/envelope types, IPC message schema (serde)
│   ├── knowledge/              — file-backed KV store (flat JSON under $ORGANISM_HOME)
│   ├── cortex/                 — error classifier + style profile builder
│   ├── daemon/                 — bin `organism` (event bus, IPC, sensors, subscribers)
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── event_bus.rs
│   │   │   ├── ipc.rs         — Unix socket RPC handler
│   │   │   ├── metrics.rs     — SharedMetrics snapshot persistence
│   │   │   ├── ollama_subscriber.rs
│   │   │   └── sensors/       — file watcher; terminal events arrive via IPC
│   │   └── tests/
│   │       ├── profile_refresh_test.rs
│   │       └── ollama_integration_test.rs
│   └── client/                 — bin `organism-cli`
│       └── src/
│           ├── main.rs
│           └── cmd_stats.rs
├── scripts/                    — install.sh, uninstall.sh, quick-install.sh, zsh hook
└── tests/                      — workspace integration tests
```

Build order: `protocol` → `knowledge` → `cortex` → `daemon` (and `client`).
`daemon` and `client` are binaries; `protocol`, `knowledge`, `cortex` are libraries.

---

## 2. IPC Protocol

**Transport:** Unix domain socket at `$ORGANISM_HOME/daemon.sock`
(default `~/.organism/daemon.sock`). One request per connection; connection
closes after response. No keep-alive, no auth tokens, no TCP fallback.

**Envelope format (newline-delimited JSON):**
```json
{ "id": "uuid4", "method": "status", "payload": {} }
```

Response:
```json
{ "id": "uuid4", "ok": true, "payload": { ... } }
```

Error response:
```json
{ "id": "uuid4", "ok": false, "payload": { "error": "reason" } }
```

**Supported methods (as of M17):**

| Method | Request payload | Response payload |
|--------|----------------|-----------------|
| `status` | `{}` | `{ "awake": bool, "uptime_s": u64, "pid": u32 }` |
| `emit-terminal` | `TerminalEvent` fields | `{ "ok": true }` |
| `log` | `{ "limit": u32 }` | `{ "events": [...] }` |
| `sleep` | `{}` | `{ "ok": true }` |
| `wake` | `{}` | `{ "ok": true }` |
| `errors` | `{ "limit": u32 }` | `{ "errors": [...] }` |
| `suggest` | `{ "error_key": str, "force": bool }` | `{ "text": str, "cached": bool }` |
| `apply` | `{ "error_key": str, "mode": "dry"|"stage" }` | apply result fields |
| `feedback` | `{ "error_key": str, "verdict": str, "note": str? }` | `{ "ok": true }` |
| `style` | `{}` | `StyleProfile` JSON |
| `profile` | `{ "params": ProfileRequest }` | `StyleProfile` JSON |
| `metrics` | `{}` | `Metrics` JSON |
| `doctor` | `{}` | `{ "daemon": str, "socket": str }` |

Verdict strings for `feedback`: `"accept"`, `"reject"`, `"ignore"`, `"applied"`.

---

## 3. Knowledge Store Schema

**Backend:** flat JSON files under `$ORGANISM_HOME/knowledge/` (default `~/.organism/knowledge/`).
No native deps, no RocksDB, no SQLite. Each record is one file.

**Key → filename mapping:** `:` replaced with `_`.

| Key prefix | Filename pattern | Type |
|-----------|-----------------|------|
| `error:<hash>` | `error_<hash>.json` | `ErrorRecord` |
| `suggestion:<hash>` | `suggestion_<hash>.json` | `SuggestionRecord` |
| `accepted:<hash>` | `accepted_<hash>.json` | `AcceptedSuggestion` |
| `feedback:<hash>` | `feedback_<hash>.json` | `FeedbackRecord` |
| `pattern:<hash>` | `pattern_<hash>.json` | `PatternRecord` |
| `style_profile:current` | `style_profile_current.json` | `StyleProfile` |
| `fix:<hash>` | `fix_<hash>.json` | `FixRecord` |

All types live in `crates/knowledge/src/types.rs`. All have `schema_v: u32`
(default 1) and `#[serde(default = "default_schema_v")]` for backward compat.

**Key types:**

```rust
// ErrorRecord — one per unique error signature
pub struct ErrorRecord {
    pub tool: String,       // "rustc", "npm", "python", etc.
    pub kind: String,       // "E0599", "MODULE_NOT_FOUND", etc.
    pub hash: String,       // sha256 hex of normalized error text
    pub raw_excerpt: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub occurrences: u64,
    pub last_command: String,
    pub schema_v: u32,
}

// FeedbackRecord — user verdict on a suggestion
pub struct FeedbackRecord {
    pub error_hash: String,
    pub suggestion_hash: String,
    pub verdict: Verdict,   // Accepted | Rejected | Ignored | Applied
    pub note: Option<String>,
    pub ts: DateTime<Utc>,
    pub schema_v: u32,
}

// AcceptedSuggestion — immutable snapshot of text at acceptance time
// Separate from suggestion_<hash>.json (which can be regenerated)
pub struct AcceptedSuggestion {
    pub suggestion_hash: String,
    pub error_hash: String,
    pub text: String,
    pub ts: DateTime<Utc>,
    pub schema_v: u32,
}

// StyleProfile — built from feedback history by cortex::build_profile
pub struct StyleProfile {
    pub feedback_count: u32,
    pub accept_rate_overall: f32,
    pub by_tool: HashMap<String, ToolStats>,
    pub by_block_kind: HashMap<String, BlockStats>,  // "patch"|"shell"|"note"
    pub preferred_terseness: Terseness,              // Concise|Standard|Verbose
    pub top_accepted_phrases: Vec<String>,
    pub top_rejected_phrases: Vec<String>,
    pub generated_at: DateTime<Utc>,
    pub schema_v: u32,
}
```

**Verdict serde:** PascalCase on disk (`"Accepted"`, `"Rejected"`, `"Ignored"`, `"Applied"`).
IPC wire format uses lowercase (`"accept"`, `"reject"`, `"ignore"`, `"applied"`) mapped
in `ipc.rs`. Do NOT add `rename_all = "snake_case"` to `Verdict` — breaks existing on-disk data.

---

## 4. Error Classification (cortex crate)

`crates/cortex/src/classify.rs` — regex-based, no ML:

1. Match `stderr_snippet` against per-tool regex banks (rustc, npm, python, shell, go, etc.)
2. Extract `(tool, kind)` pair
3. Hash normalized error text → `hash` (sha256 hex)
4. Key: `error:<hash>` — stored as `ErrorRecord`; duplicate hashes bump `occurrences`

PII redaction (`crates/cortex/src/redact.rs`):
- Strips emails, 40-char+ hex tokens, UUIDs, remote URLs before storing
- Applied to `raw_excerpt` before classification

---

## 5. Style Profile (cortex crate)

`crates/cortex/src/style.rs` — `build_profile(feedback, accepted_text, tool_for_hash, block_kind_for_suggestion)`:

- **accept_rate_overall**: weighted; `Applied` counts 2× in both numerator and denominator
  so rate stays in [0, 1]
- **by_tool**: `ToolStats { accepts, rejects }` per tool name from ErrorRecord
- **by_block_kind**: `BlockStats` per `"patch"|"shell"|"note"` (classified from accepted text)
- **preferred_terseness**: from avg line count of accepted suggestions (<8 → Concise, ≤20 → Standard, >20 → Verbose)
- **top_accepted_phrases**: top-10 2-gram + 3-gram phrases from accepted suggestion text, after stopword filtering
- Rebuilt after every N feedback events (ORGANISM_PROFILE_REFRESH_EVERY, default 10), rate-limited (ORGANISM_PROFILE_REFRESH_MIN_INTERVAL_MS, default 60s)

**Block kind classification** (`classify_block_kind`):
- `` ```diff `` / `` ```patch `` or starts with `--- ` → `"patch"`
- `` ```shell `` / `` ```bash `` / `` ```sh `` or starts with `$ ` → `"shell"`
- everything else → `"note"`

---

## 6. Ollama Integration (daemon crate)

`crates/daemon/src/ollama_subscriber.rs`:

- Spawns tokio task that subscribes to EventBus
- On new `ErrorRecord` write, calls `organism-ollama` client (POST `/api/generate`)
- Persists response as `suggestion_<error_hash>.json`
- Calls `maybe_notify()` to fire a desktop notification if:
  - Tool's accept rate in StyleProfile ≥ 0.70 (notification gate)
  - Error hash not already notified this session (per-session HashSet dedup)
- Gated by `OLLAMA_ENABLED=1` env var (default: disabled)
- Best-effort: logs warn on Ollama errors, never crashes daemon

**Ollama crate** (`crates/ollama/`):
- `OllamaClient { base_url, model, http: reqwest::Client }`
- `async fn generate(&self, prompt: &str) -> Result<String>`
- POST `{base_url}/api/generate` with `{ "model": ..., "prompt": ..., "stream": false }`
- Defaults: `OLLAMA_BASE_URL=http://127.0.0.1:11434`, `OLLAMA_MODEL=qwen2.5-coder:7b`
- Prompt template (M11 few-shot): includes StyleProfile header + kNN-selected accepted examples

---

## 7. Apply Workflow

`ipc.rs` `"apply"` handler:

1. Load `ErrorRecord` for `error_key`
2. Load `suggestion_<error_hash>.json` (cached LLM output)
3. Parse suggestion text into blocks: patch blocks, shell blocks, note blocks (M7 multi-block)
4. Build apply plan: `{ plan_kind: "patch"|"shell"|"note", ... }`
5. If `mode = "stage"`:
   - patch: write unified diff to `/tmp/organism_patch_<hash>.patch`
   - shell: copy to clipboard via `pbcopy` (macOS) or `xclip`/`xsel` (Linux)
   - note: print only
6. Return plan details to CLI

CLI (`cmd_apply` in `main.rs`):
- After staging, if `plan_kind != "note"` AND `stdin.is_terminal()`:
  - Prints `"Did you apply this patch? [y/N]: "`
  - If `y` → sends `feedback` IPC request with `verdict: "applied"`
  - Daemon records `Verdict::Applied` (2× weight in profile)

---

## 8. Metrics

`crates/protocol/src/metrics.rs`:

```rust
pub struct Metrics {
    pub suggestions_total: u64,
    pub suggestions_cached: u64,
    pub feedback_accept: u64,
    pub feedback_reject: u64,
    pub feedback_applied: u64,   // Verdict::Applied (serde default=0 for backward compat)
    pub by_tool: HashMap<String, ToolMetrics>,
    pub since: DateTime<Utc>,
    pub prompt_version: String,
}
```

Persisted as `metrics_snapshot.json` in `$ORGANISM_HOME`. `cmd_stats --capture-baseline`
copies snapshot to `metrics_baseline.json` for delta tracking.

---

## 9. Testing Conventions

- `#[cfg(test)] mod tests { ... }` at end of each `src/*.rs`
- Integration tests in `crates/<crate>/tests/*.rs`
- `tempfile::TempDir` for all filesystem tests (never `~/.organism/`)
- `#[tokio::test]` for async tests, `#[test]` for sync
- `#[serial]` (from `serial_test` crate) on tests that use global state (e.g. `REFRESH_STATE`)
- wiremock for Ollama HTTP tests

Clippy: `cargo clippy --workspace -- -D warnings` must pass. No `#[allow(...)]`
without justification comment.

---

## 10. CI (`.github/workflows/ci.yml`)

Jobs: `build-and-test` (ubuntu + macos matrix), `clippy`, `fmt`.
All run `--locked`. `fmt` job runs `cargo fmt --all -- --check`.

---

*Updated through M17 (2026-05-15). For per-task notes and gotchas, see `LEARNINGS.md`.*
