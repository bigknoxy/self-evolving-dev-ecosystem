# LEARNINGS.md — Self-Evolving Dev Ecosystem

> Append after every task, compiler error, or non-obvious discovery.

## Format
```
## Learning NNN — YYYY-MM-DD: TASK-NNN — <title>
**Problem:** ...
**Root cause:** ...
**Fix:** ...
**Prevention:** ...
```

---

## Pre-populated Gotchas

### G-001: Workspace vs crate Cargo.toml deps
Shared dependencies belong in root `Cargo.toml` under `[workspace.dependencies]`.
In crate `Cargo.toml`, reference as: `tokio = { workspace = true }`.
Never duplicate `version = "..."` in both workspace and crate — it causes conflicts.

### G-002: broadcast::Receiver is not Clone
`tokio::sync::broadcast::Receiver<T>` cannot be cloned.
To give multiple tasks the same stream, each must call `bus.subscribe()` to get its own receiver.
Store `Arc<EventBus>` (not the receiver) in shared state.

### G-003: chrono serde feature
`chrono::DateTime<Utc>` serialization requires `chrono = { features = ["serde"] }`.
Without this feature flag, `#[derive(Serialize, Deserialize)]` on structs with `DateTime` fails.

### G-004: tokio::test vs test
`#[test]` for synchronous tests, `#[tokio::test]` for async tests.
Calling async functions from `#[test]` causes: "Cannot call a runtime from within a runtime".
Use `#[tokio::test]` for any test that uses `await`.

### G-005: Dead code warnings in workspace builds
`cargo clippy -- -D warnings` flags unused public items as warnings (elevated to errors).
For stub implementations that will be used later, either:
a) Add `#[allow(dead_code)]` with a comment explaining when it will be used, OR
b) Implement a minimal consumer in the same crate (even just a test that calls it).
Prefer option (b) — it also serves as a usage example.

### G-006: tempfile::TempDir drop cleanup
`TempDir` is automatically deleted when it goes out of scope (RAII).
In tests: `let tmp = TempDir::new().unwrap(); let path = tmp.path().to_path_buf();`
If `tmp` drops before you're done using `path`, the directory is gone.
Keep `tmp` alive for the duration of the test by binding it to a named variable.

### G-007: File-based KV store key encoding
Keys use `:` as separator (e.g., `fix:abc123`).
File names cannot contain `:` on some systems (Windows), so keys are encoded by replacing `_` on write.
The encoding must be symmetric: `fix:abc123` → `fix_abc123.json` → key `fix:abc123`.
The current implementation uses `replace([':', '/'], "_")` — this is lossy if keys contain `_`.
Limitation accepted for now; avoid using `_` in key subcomponents.

---

*No task learnings yet — add them as you work.*
## Completed: TASK-009 — 2026-04-29
## Completed: TASK-010 — 2026-04-29
## Completed: L1-W0 smoke loop — 2026-04-29

## Learning 011 — 2026-04-29: L1-W0 — daemon binary-only crate forces inline EventBus in integration tests
**Problem:** The smoke integration test in `crates/daemon/tests/smoke_test.rs` cannot `use organism_daemon::event_bus::EventBus` because `organism-daemon` is a binary-only crate (no `lib.rs`), so its modules are not externally importable.
**Root cause:** `crates/daemon/Cargo.toml` declares only `[[bin]] name = "organism"`. Tests in `tests/*.rs` of a bin-only crate cannot reach `mod event_bus;` declared in `main.rs`.
**Fix:** Replicate the tiny `EventBus` wrapper inline inside the test file (matching the existing pattern in `tests/integration_test.rs`). The wrapper is a thin facade over `tokio::sync::broadcast`, so duplication risk is low. No source-tree changes needed.
**Prevention:** When daemon graduates to needing shared internals across multiple integration tests or external consumers, split it into `organism-daemon` (lib) + `organism` (bin re-exporting `Daemon`/`EventBus`). Until then, document that integration tests target public crate APIs (protocol/knowledge/cortex) and stub the bus locally.

## Completed: IPC (Unix socket RPC) — 2026-04-29

### Learning: stale socket cleanup + parent dir
`UnixListener::bind` fails with `Address already in use` if a previous daemon
crashed leaving the socket file behind. Always `remove_file` (ignore ENOENT)
before binding, and `create_dir_all` the parent first.

### Learning: tokio::select! shutdown ordering
Shutting the event loop down on `ctrl_c` requires putting `tokio::signal::ctrl_c()`
on the same `select!` as `bus.recv()`. Either branch firing must `break`; otherwise
a closed bus would loop forever and SIGINT would only land between iterations.

### Learning: #[path]-mounted modules trip dead_code
Test files that re-mount the daemon binary's `src/*.rs` via `#[path = ...]`
trigger `dead_code` errors under `-D warnings` for any items the test does not
exercise (e.g. `Daemon::new`, `run_event_loop`, the `knowledge` field).
Annotate the mounted modules with `#[allow(dead_code)]` at the `mod` level
rather than scattering `#[allow]` through the source.

### Learning: newline-delimited JSON framing is enough for one-shot RPCs
Each request = one line, response = one line, server then `shutdown()`s the
write half so the client's `read_line` returns. No length-prefix or MessagePack
framing needed at this scale; serde_json round-trips the `Envelope` cleanly.

## Completed: L1-W2 terminal sensor — 2026-04-29

Wired zsh terminal events into the running daemon via a new IPC `event`
method, an `organism-cli emit-terminal` subcommand, and a `preexec`/`precmd`
shell hook script.

### Zsh hook gotchas worth remembering

- **`$?` capture order in `precmd`**: the FIRST line of precmd must be
  `local ec=$?`. Any earlier statement (even `[[ -z VAR ]]`) clobbers `$?`
  and you record the wrong exit code for the user's command.
- **`preexec` arguments**: zsh passes the *expanded* command line as `$1`
  (also `$2` = sans-aliases, `$3` = full multiline). Use `$1` for "what the
  user actually typed".
- **`&` vs `&!`**: plain `&` backgrounds but the job stays attached to the
  shell's job table (shows in `jobs`, may print "done" lines, blocks exit
  on lingering jobs). `&!` backgrounds AND disowns in one step — required
  for a fire-and-forget sensor that must never affect the prompt.
- **`{ ... } &!` vs `( ... ) &!`**: braces are an in-shell group and
  inherit traps cheaply; parens fork a subshell. Braces are lighter.
- **Double-source guard**: zshrc gets sourced more than once in some
  setups (sub-shells, `exec zsh`). A `__ORGANISM_HOOK_LOADED` sentinel
  prevents stacking the hook in `preexec_functions` twice, which would
  fire the CLI twice per command.
- **`add-zsh-hook` autoload**: prefer it over manually appending to
  `preexec_functions` — it deduplicates and survives reloads. Fall back
  to the manual array push only if the autoload isn't available.
- **Silent-on-failure**: redirecting both stdout and stderr to `/dev/null`
  inside the background block is mandatory; otherwise a missing daemon
  emits "daemon not running" lines that interleave with the user's next
  prompt.

### Daemon-side wart (not a bug, worth noting)

Events injected via the IPC `event` path get recorded **twice** in the
ring buffer / `event_count`: once in `dispatch()` per the spec, and once
again when the same event fires through `run_event_loop`'s broadcast
subscriber. If we ever want exact counts, either drop the `record_event`
in dispatch OR have `run_event_loop` skip events tagged as "already
recorded". For Level 1 observability the duplication is harmless.

## Learning 012 — 2026-04-29: L1-W2 — Double-record bug
**Problem:** event_count incremented twice per IPC-injected event.
**Root cause:** ipc::dispatch called record_event AND published to bus; run_event_loop subscribed and also recorded.
**Fix:** Removed record_event from run_event_loop. Producer side (ipc/sensors) is canonical recorder; loop is consumer hook for cortex/effector at L1+.
**Prevention:** One-writer rule for event_count/recent_events. Document in daemon.rs comment.
## Completed: L2-W0 TerminalEvent fields — 2026-04-29

Added native exit_code: Option<i32> and duration_ms: Option<u64> to TerminalEvent with #[serde(default)] for back-compat. Dropped the snippet-encoding hack from organism-cli emit-terminal. The daemon binary is named 'organism' (not 'organism-daemon') in target/release/. Existing TerminalEvent struct literals across the workspace (lib.rs, smoke_test.rs, integration_test.rs, both event_ingest_test cases) all required updates since the struct uses positional/named field literals without ..Default::default(); serde defaults only handle deserialization, not Rust construction.


## Completed: L2-W1b error classifier — 2026-04-30

- Added `organism_cortex::error_classifier` with `classify(cmd, exit_code, stderr) -> Option<ErrorSignature>`. Rules in fixed precedence: rustc `E####`, npm ERR!, Python `Traceback` + `*Error:`, shell `command not found`, then unknown nonzero exit. `Some(0)` returns None; `(None, None)` returns None.
- Hash uses `std::collections::hash_map::DefaultHasher` over (tool, kind, first_64_chars). Deterministic within a process; not guaranteed cross-version stable. Adequate for session-local dedup; swap for `sha2` if cross-process stability needed.
- Knowledge gained `ErrorRecord` with INLINED signature fields (tool/kind/hash/raw_excerpt) to avoid a knowledge → cortex dep cycle. Accessors: `put_error`, `get_error`, `list_errors`, key prefix `error:`.
- `KnowledgeStore::list_keys` reverse-maps `_` back to `:`. Hex hashes from DefaultHasher (`{:016x}`) contain only `[0-9a-f]`, so no `_` vs `:` ambiguity.
- Daemon spawns `error_subscriber::run` from `main.rs` after the file watcher block (does not modify `sensors/file.rs`). Subscriber filters Terminal events with `exit_code != Some(0)`, calls `classify`, upserts `ErrorRecord`. `occurrences` increments via `saturating_add(1)` on duplicate hash.
- Integration test imports daemon modules with `#[path]` (same pattern as existing `event_ingest_test.rs`).

## Completed: L2-W2 install + LaunchAgent — 2026-04-30

## Completed: L2 final acceptance smoke — 2026-04-30
End-to-end verified: install.sh (sandboxed HOME) → daemon launch → file watcher → emit-terminal → classifier → ErrorRecord persisted with occurrence increment. Sleep/wake gating verified.
