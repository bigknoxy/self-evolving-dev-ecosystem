# Developer Superorganism — Implementation Details

> Detailed engineering appendix for the "Developer Superorganism" daemon project. Covers crate layout, IPC contracts, plugin API, knowledge store schema, LLM integration choices, testing, CI, performance budgets, safety, and deployment.

---

## 1. Repo Layout (Rust workspace)

```
self-evolving-dev-ecosystem/
├── crates/
│   ├── protocol/           # IPC types & shared message formats (serde)
│   ├── knowledge/          # file-backed KV store (no RocksDB; flat JSON under $ORGANISM_HOME)
│   ├── cortex/             # pattern engine + error classifier
│   ├── daemon/             # main daemon binary (bin name: `organism`)
│   │   └── src/
│   │       ├── main.rs
│   │       ├── daemon.rs
│   │       ├── event_bus.rs
│   │       ├── error_subscriber.rs
│   │       ├── ipc.rs
│   │       └── sensors/    # file watcher; terminal events arrive via IPC
│   └── client/             # `organism-cli` binary
├── scripts/                # install.sh, uninstall.sh, zsh hook, LaunchAgent plist
├── tests/                  # workspace-wide integration tests
└── Cargo.toml              # workspace manifest
```

Plugins (`react_plugin`, `python_plugin`) and effectors are L3+ scope — not present today.

Language: Rust 1.70+ (stable). Use tokio async runtime, serde for structured messages, anyhow/thiserror for errors, tracing/tracing-subscriber for logging.

---

## 2. Core Concepts & Contracts

- Event: a typed struct describing an observation (TerminalInput, FileChange, GitEvent, ProcessSpawn, NetworkCall)
- Thought: an internal inference produced by Cortex from events (e.g., "likely working on React auth feature")
- Action: a change the daemon can perform (AutoFixPatch, SetEnvVar, RunCommand, FormatFile)
- KnowledgeNode: persistent record in RocksDB capturing patterns, learned style, fixes

All messages defined in `protocol` crate and serialized with JSON over Unix socket (or named pipe on Windows). Use a versioned envelope:

```json
{
  "v": 1,
  "type": "Event|Thought|Action|Ack|Error",
  "id": "uuid4",
  "ts": "2026-04-26T...Z",
  "payload": { ... }
}
```

---

## 3. IPC & Client Protocol

Transport choices:
- Local Unix domain sockets (preferred on macOS / Linux) at `$ORGANISM_HOME/daemon.sock` (default `~/.organism/daemon.sock`)
- Fallback: TCP loopback on `127.0.0.1:8765` restricted to localhost

Protocol characteristics:
- JSON envelope (versioned) for forward/backward compatibility
- Message types: Request/Response for RPC style; Events for pub/sub
- Keep-alive heartbeat messages every 30s
- Authentication: file-based token stored at `~/.organism/token` (random 256-bit key). Clients must present token for privileged RPCs (actions, undo)

Example request (client → daemon):
```json
{ "v":1, "type":"Request", "id":"...", "method":"status.get", "params":{} }
```

Example response:
```json
{ "v":1, "type":"Response", "id":"...", "result": {"status":"ok","uptime_s":1234} }
```

Streaming: subscribe to events via `events.subscribe` and receive Event envelopes pushed by daemon.

---

## 4. Event Schema (protocol crate)

Key event payloads (Rust structs serialized with serde):

- TerminalEvent
  - ts, pid, cwd, command_line, stdout_snippet, stderr_snippet, keystroke_rate
- FileEvent
  - ts, path, event_type (create/modify/delete), size_bytes, owner_uid
- GitEvent
  - ts, repo_path, branch, head_sha, commit_msg, author
- ProcessEvent
  - ts, pid, cmd, exit_code, cpu_ms, mem_kb
- ClipboardEvent
  - ts, content_type, content_snippet

All events carry `context` map with optional keys: project_id, detected_stack, last_error_signature

---

## 5. Knowledge Store (RocksDB) Schema

Design: use prefixed keys for separation. Keys are binary: prefix:u8 + subkey

Prefixes:
- 0x01: project:meta:<project_id> -> JSON value
- 0x02: fix_db:signature:<hash> -> FixRecord JSON
- 0x03: style:profile:<user_id> -> StyleProfile JSON
- 0x04: patterns:<pattern_id> -> Pattern JSON
- 0x05: stats:metric:<metric_name> -> Timeseries (protobuf or JSON array)

Example FixRecord:
```json
{
  "id":"uuid4",
  "signature_hash":"sha256 of error snippet",
  "patch":"diff unified or patch file",
  "confidence":0.92,
  "applied_count":3,
  "last_applied":"2026-04-20T...Z",
  "source": "learned|manual|imported"
}
```

RocksDB options:
- Use RocksDB column families to separate large namespaces
- Enable LZ4 compression and block cache
- Periodic compaction scheduled at idle times

Backup & encryption:
- Optionally encrypt DB with user key derived from SSH agent or passphrase using per-value AES-GCM wrapping keys (managed by knowledge crate)

---

## 6. Plugin System (ABI & Security)

Plugin types: dynamic libraries loaded at runtime or sandboxed subprocesses.

Preferred design: Initially implement sandboxed subprocess plugins (separate process) communicating over stdin/stdout JSON RPC; later add dynamic .dylib loading with C ABI.

Plugin API (JSON RPC over stdio) — minimal interface:
- `handshake`: plugin -> daemon supplies metadata (name, version, capabilities)
- `on_event(event) -> responses[]` : plugin receives events and can emit zero or more responses (thoughts, actions, logs)
- `on_command(command) -> result` : invoked when user triggers plugin command

Security:
- Plugins run with restricted privileges by default (no network). If plugin needs network, user must approve and sign plugin.
- Signed plugin model: user can only install plugins from `~/.organism/plugins/` and must approve unknown plugin fingerprints.

Example handshake response:
```json
{"name":"react_plugin","version":"0.1","capabilities":["lint","component_suggest"]}
```

---

## 7. Cortex / Pattern Engine

Responsibilities:
- Aggregate events into sessions by project_id and time window
- Extract features: commit frequency, naming patterns, code token distributions, error signatures
- Detect recurring sequences (pattern mining) using lightweight algorithms:
  - Frequent sequence mining (PrefixSpan) on recent n-grams of actions
  - TF-IDF on tokenized identifiers to build a "vocabulary" of user naming conventions
- Output: Pattern objects with frequency and example occurrences, persisted in knowledge store

Model lifecycle:
- Models are small, deterministic, tuned for on-device inference, stored as JSON; retraining occurs in background when new data reaches thresholds (e.g., 1k events)

Confidence calculation: use bootstrapped statistics to estimate real effect sizes. E.g., experiment improvement / noise_floor.

---

## 8. Codec Engine (Digital Twin)

Phases:
- V0: LLM-driven code generation via local Ollama model with constrained prompts and retrieval augmentation (local README + commit msgs + style profile)
- V1: Style-conditional generation (prepend style profile to prompt)
- V2: Patch generation and verification: generate patch -> apply in temp branch -> run tests/lint -> if passes, propose/apply

Prompting rules:
- Deterministic (temperature 0.0) for core scaffolding unless user enables creative mode
- Provide examples of user's previous code (up to N files) as context
- Include explicit constraints: target file, API shape, test expectations

Safety:
- Generated code must pass local linter and basic test harness before being auto-applied under `assist` trust-level
- Under `ask` trust-level, generate patch and present diff for confirmation

---

## 9. Actions & Undo Model

All actions performed by daemon are logged to `~/.organism/actions.log` JSONL with fields:
- action_id, ts, actor (daemon/plugin), type, payload, backup_path (if destructive), verification_result

Undo semantics:
- For file edits: store pre-image in `~/.organism/backups/<action_id>.tar.gz` and allow restore
- For env changes: store previous env and shell profiles, restore on undo
- For command runs with side effects (e.g., package install): provide best-effort rollback (e.g., uninstall), but mark as non-reversible and require explicit user consent

CLI: `organism undo --id <action_id>` or `organism undo --latest 1`

---

## 10. Safety Levels & Configuration

Trust levels (configurable per-host and per-project):
- `observer` — observe-only, no suggestions
- `ask` (default) — suggest and require acceptance
- `assist` — auto-apply low-risk fixes (formatting, lint autofixes)
- `autonomous` — perform any action within confidence thresholds
- `uncaged` — full autonomy (dangerous)

Config file `~/.organism/config.toml` contains keys:
- sensors: list of sensors to enable
- trusted_plugins: fingerprints
- trust_level: ask
- max_concurrent_plugins: 4
- llm: { backend: 'ollama', model: 'vicuna-13b', allow_remote: false }

Kill switch:
- `organism sleep` writes a lock file and stops sensors quickly
- `organism emergency-stop` kills the daemon immediately

---

## 11. Testing Strategy

Unit tests (per crate) using `cargo test`.
- `daemon` crate: mock sensors with deterministic event feeds
- `protocol` crate: serialization roundtrip tests
- `cortex` crate: pattern detection tests with synthetic data
- `knowledge` crate: RocksDB in-tempdir tests, migration tests

Integration tests (workspace-level):
- `tests/integration/test_end_to_end.rs` spawn daemon in tempdir, connect client, send events and assert actions emitted
- `tests/integration/test_plugin_handshake.rs` spawn plugin subprocess and validate handshake

Fuzzing & property tests:
- Use `proptest` for RPC boundary checks (malformed messages)
- Use `cargo-fuzz` for compact critical sections (parsers)

Performance & Load tests:
- `tests/load/test_event_throughput.rs` measure sustained events/sec (target 2k events/s on dev machine)

CI: run tests on GitHub Actions with matrix for stable Rust versions.

---

## 12. CI/CD (`.github/workflows/ci.yml`)

```yaml
name: Superorganism CI
on: [push, pull_request]
jobs:
  build-and-test:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        rust: [1.70.0]
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust
        uses: dtolnay/rust-toolchain@v1
      - name: Cache cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
      - name: Cargo build
        run: cargo test --workspace --all-features --verbose

  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - run: cargo clippy --all-targets --all-features -- -D warnings
```

---

## 13. Observability & Telemetry

Local logs:
- `~/.organism/logs/daemon.log` (rotating)
- `~/.organism/actions.log` (JSONL audit trail)

Metrics:
- Expose Prometheus metrics on a unix-socket file (textfile) or local TCP port when enabled: events_received, actions_executed, avg_processing_ms, plugin_count

Optional telemetry (opt-in):
- Hash of machine id, counts of events, error rates — explicit opt-in in config and separate `--telemetry enable` command

---

## 14. LLM Integration Details

Primary local-first approach: Ollama or llama.cpp backends. Provide an abstraction layer so the backend can be swapped.

Requirements:
- Offline capability with local models
- Prompt templates stored in `~/.organism/prompts/` and versioned
- Retrieval augmentation: fetch relevant files (README, tests, last 10 commits) and attach to prompt
- Rate limiting & batching: do not send long prompts at high frequency

Security: never send private repo content to remote LLMs unless `allow_remote=true` in config

---

## 15. Packaging, Installation & Upgrades

Install options:
- Cargo install for developers: `cargo install --path daemon/ --bin organism-cli`
- Prebuilt releases (macOS/Linux): provide tar.gz with binary and `organism-install` script

Install script actions:
- Create `~/.organism/` directories with proper permissions
- Generate `~/.organism/token` and instruct user to add it to `$XDG_RUNTIME_DIR` for clients
- Optionally add shell hooks (`.zshrc`/`.bashrc`) to enable `preexec`/`postcmd` integration

Auto-updates:
- Self-update via GH Releases (download new release and replace binary) with signature verification

---

## 16. Performance Budgets

| Operation | Target |
|-----------|--------|
| Daemon steady memory | < 150 MB |
| Event processing latency (p50) | < 20 ms |
| Event processing latency (p95) | < 150 ms |
| Plugin handshake time | < 200 ms |
| Style profile computation (background) | < 2s per 1000 files |

Profiling tools: `perf`, `tokio-console`, and flamegraphs.

---

## 17. Security & Privacy Considerations

- Local-only defaults
- All persisted data encrypted at rest if user enables `encrypt_db=true` via key derived from passphrase
- Plugin approval workflow for third-party plugins
- Audit trail: every action has an auditable JSONL entry stored locally
- Sensitive content redaction: by default do not store clipboard or file content longer than 1KB — store hashed signatures instead

---

## 18. Migration & Backups

- Knowledge DB version recorded in `~/.organism/schema_version`
- `organism backup --dest /path/to/backup.tar.gz` creates encrypted backup of DB + actions log
- `organism restore --src /path/to/backup.tar.gz` restores state (interactive)

---

## 19. UX Onboarding & Commands

- `organism install` — run once, sets up repo, token, shell hooks (opt-in)
- `organism start` — start daemon (or systemctl/launchd service)
- `organism status` — prints health and active sensors
- `organism sleep` / `organism wake` — pause/resume
- `organism suggest --one` — request a suggestion for current cwd
- `organism apply --id <suggestion-id>` — apply suggested patch (requires proper trust level)
- `organism plugin install <path-or-url>` — installs plugin with fingerprint check

---

## 20. Implementation Roadmap (Immediate Tasks)
1. Implement `protocol` crate with versioned message types and JSON serialization tests.
2. Build `daemon` skeleton: event bus, simple terminal sensor (reads a synthetic event file) and client RPC for status.
3. Implement `knowledge` crate with RocksDB basic CRUD and migrations.
4. Implement plugin subprocess API and a sample `react_plugin` as proof-of-concept.
5. Integrate local Ollama client wrapper and a safe prompt execution pipeline.
6. Add thorough unit + integration tests and set up CI.

---

End of Developer Superorganism Implementation Details
