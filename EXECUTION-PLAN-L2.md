# L2 EXECUTION PLAN — Sensors + Classifier + Install

## Waves

### W0: Protocol — TerminalEvent fields (unblocks classifier)
- Add `pub exit_code: Option<i32>`, `pub duration_ms: Option<u64>` to `TerminalEvent`.
- Update CLI `emit-terminal` to populate them natively (drop snippet hack).
- Update zsh hook to pass them via flags (already does).
- Roundtrip test: serialize → deserialize → fields preserved.
- Backward-compat: `#[serde(default)]` so old envelopes still parse.

### W1a: File watcher (`notify` crate)
- New module `crates/daemon/src/sensors/file.rs`.
- `pub async fn watch(bus: Arc<EventBus>, root: PathBuf, shutdown: oneshot::Receiver<()>) -> Result<()>`.
- Debounce 200ms (collapse rapid saves).
- Filter: ignore `target/`, `.git/`, dotfiles by default.
- Emits `OrganismEvent::File { ts, path, kind: Create|Modify|Delete }` (add `FileEvent` to protocol if missing).
- Spawn from `daemon::run_event_loop` alongside ipc::serve.
- Tests: TempDir, watch, touch file, assert event received within 1s; assert ignored paths skipped.

### W1b: Error classifier
- New crate? No — add to cortex: `crates/cortex/src/error_classifier.rs`.
- `pub fn classify(cmd: &str, exit_code: Option<i32>, stderr: Option<&str>) -> Option<ErrorSignature>`.
- `ErrorSignature { tool: String, kind: String, hash: String }` — deterministic hash for dedup.
- Built-in patterns (regex): rust compile errors `error\[E\d+\]`, npm `npm ERR!`, python `Traceback`, generic `command not found`.
- Subscribe in daemon: on TerminalEvent with exit_code != 0, classify → store as PatternRecord (or new ErrorRecord) in knowledge.
- Tests: 1 test per pattern (4+), 1 negative test (clean output → None), 1 unknown-tool test.

### W2: Install script + LaunchAgent
- `scripts/install.sh`:
  - Build release.
  - Copy binaries to `~/.local/bin/` (or `/usr/local/bin/` if writable).
  - Append source line to `~/.zshrc` (idempotent — guard with marker comment).
  - Install LaunchAgent plist (macOS only).
- `scripts/com.organism.daemon.plist` (LaunchAgent template):
  - `RunAtLoad`, `KeepAlive`, `StandardOutPath` to `~/.organism/daemon.log`.
- `scripts/uninstall.sh`: reverse.
- Tests: dry-run mode prints actions without executing. Bash `-n` syntax check. Plist `plutil -lint`.

## Coverage gate per wave

- `cargo test --workspace` 0 failures
- `cargo clippy --workspace --all-targets -- -D warnings` 0 warnings
- New code: every `pub fn` has ≥1 test, every `Result::Err` arm exercised
- Integration test per cross-crate boundary

## Final acceptance

- `./scripts/install.sh --dry-run` prints expected actions
- LaunchAgent plist passes `plutil -lint`
- Real terminal session: source hook, run failing `cargo build`, `organism-cli log` shows event with exit_code, knowledge store has classified ErrorRecord
- All learnings appended

## Out of scope

- Ollama integration (L3)
- Real digital-twin generation (L4)
- Code suggestion UI (L2 followup)
- Windows/Linux equivalents (LaunchAgent macOS-only this pass)
