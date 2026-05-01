# Plan: L3.5 — Effector Seed (`organism-cli apply`)

**Audience:** junior dev driving a small local LLM (qwen2.5-coder:7b or similar). Each task is self-contained, files are explicit, acceptance is binary. No design decisions left open.

## Goal

Bridge from **observe-only** (L3) to **assisted action** (L4). User sees a cached LLM suggestion, runs `organism-cli apply <hash>`, and the daemon emits an actionable artifact (patch on clipboard, or `git apply`-able diff file). User remains in control — daemon never writes to source files.

## Non-goals

- Autonomous code edits (that's L4).
- Multi-file refactors.
- Anything that mutates `.git/` or repo state.

---

## Architecture

```
suggestion_<hash>.json  ──parse──>  ApplyPlan { kind, payload }
                                    │
                                    ├── Patch(diff_text)    → write /tmp/organism-<hash>.patch
                                    ├── Shell(cmd)          → write to clipboard
                                    └── Note(text)          → stdout only
```

**ApplyPlan extraction = pure function** over the suggestion text. Regex-based, no LLM call. Three patterns:

1. **Patch**: text contains a fenced ```diff or ```patch block → `Patch`
2. **Shell**: text contains a fenced ```bash, ```sh, or ```zsh block → `Shell`
3. **Else**: `Note`

If suggestion has multiple blocks, pick the FIRST one. (Junior devs can iterate later; v1 = first-match.)

---

## Crate-by-crate changes

### TASK-L3.5-01 — `cortex::apply` module (parser)

**File:** `crates/cortex/src/apply.rs` (NEW). Add `pub mod apply;` to `crates/cortex/src/lib.rs`.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApplyPlan {
    Patch { diff: String },
    Shell { command: String },
    Note { text: String },
}

pub fn extract_plan(suggestion: &str) -> ApplyPlan { /* ... */ }
```

**Implementation:** scan for fenced code blocks via regex `(?s)```(\w+)?\n(.*?)````. Match language tag:
- `diff` | `patch` → `ApplyPlan::Patch`
- `bash` | `sh` | `zsh` | `shell` → `ApplyPlan::Shell`
- anything else → keep scanning
- no match → `ApplyPlan::Note { text: suggestion.to_string() }`

**Tests** (`#[cfg(test)] mod tests` at end of file, 6 tests minimum):

| Test | Input | Expected |
|------|-------|----------|
| `diff_block_extracted` | `"foo\n` ` ```diff\n-a\n+b\n``` `\n"` | `Patch { diff: "-a\n+b\n" }` |
| `patch_block_extracted` | `` ```patch\n...\n``` `` | `Patch` |
| `bash_block_extracted` | `` ```bash\necho hi\n``` `` | `Shell { command: "echo hi\n" }` |
| `sh_block_extracted` | `` ```sh\nls\n``` `` | `Shell` |
| `unknown_lang_falls_through` | `` ```python\nprint(1)\n``` `` | `Note` |
| `no_block_returns_note` | `"just text"` | `Note { text: "just text" }` |
| `first_block_wins` | bash then diff | `Shell` |

**Deps:** add `regex = "1"` to `crates/cortex/Cargo.toml` if not present.

**Acceptance:** `cargo test -p organism-cortex apply` green. `cargo clippy -p organism-cortex -- -D warnings` clean.

---

### TASK-L3.5-02 — Protocol `Apply` request/response

**File:** `crates/protocol/src/messages.rs`. Add after `SuggestResponse`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyRequest {
    pub error_key: String,
    pub mode: ApplyMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyMode {
    /// Just print plan to stdout. Default. Safe.
    Dry,
    /// Write patch to /tmp file or copy shell cmd to clipboard.
    Stage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResponse {
    pub plan_kind: String,           // "patch" | "shell" | "note"
    pub artifact_path: Option<String>, // /tmp/organism-<hash>.patch when Patch+Stage
    pub clipboard: bool,             // true if shell cmd was copied
    pub message: String,             // human-readable summary
}
```

**Tests** (`crates/protocol/src/messages.rs` tests module): roundtrip serde for `ApplyRequest`, `ApplyResponse`, and both `ApplyMode` variants.

**Acceptance:** `cargo test -p organism-protocol` green.

---

### TASK-L3.5-03 — Knowledge: load suggestion by hash (read-only helper)

**File:** `crates/knowledge/src/store.rs`. Verify `pub fn get_suggestion(&self, hash: &str) -> Result<Option<String>>` already exists (it does per L3 work). Add ONE helper:

```rust
/// Return (error_record, suggestion_text) for a hash, both optional.
pub fn load_pair(&self, hash: &str) -> Result<(Option<ErrorRecord>, Option<String>)> {
    Ok((self.get_error(hash)?, self.get_suggestion(hash)?))
}
```

**Tests:** add 2 tests — pair present, pair half-missing. Use `tempfile::TempDir`.

**Acceptance:** `cargo test -p organism-knowledge` green.

---

### TASK-L3.5-04 — Daemon `Apply` IPC handler

**File:** `crates/daemon/src/ipc.rs`. Mirror existing `"suggest"` handler shape.

1. Add `"apply"` arm in `dispatch()` matching on `req.method`.
2. Parse `ApplyRequest` from `req.params`.
3. `let suggestion = knowledge.read().await.get_suggestion(&req.error_key)?;`
4. If `None` → return `Envelope::error(&req.id, "no_suggestion", "no cached suggestion for that hash")`.
5. `let plan = cortex::apply::extract_plan(&suggestion);`
6. Branch on `plan` × `mode`:

| plan | mode | action |
|------|------|--------|
| `Note` | * | response `{ plan_kind:"note", message: text }` |
| `Patch` | `Dry` | response `{ plan_kind:"patch", message: "diff (dry)\n\n<diff>" }` |
| `Patch` | `Stage` | write `/tmp/organism-<hash>.patch`, response `{ plan_kind:"patch", artifact_path: Some(path), message: "patch written. apply with: git apply <path>" }` |
| `Shell` | `Dry` | response `{ plan_kind:"shell", message: "would run: <cmd>" }` |
| `Shell` | `Stage` | copy to clipboard via `pbcopy` (macOS) or `xclip -selection clipboard` (Linux). On failure, fall back to printing cmd. response `{ plan_kind:"shell", clipboard: true_or_false, message: "..." }` |

**Clipboard helper** (`crates/daemon/src/clipboard.rs`, NEW):

```rust
use std::io::Write;
use std::process::{Command, Stdio};

pub fn copy(text: &str) -> anyhow::Result<bool> {
    let bin = if cfg!(target_os = "macos") { "pbcopy" }
              else if cfg!(target_os = "linux") { "xclip" }
              else { return Ok(false) };
    let args: &[&str] = if bin == "xclip" { &["-selection", "clipboard"] } else { &[] };
    let mut child = match Command::new(bin).args(args)
        .stdin(Stdio::piped()).stderr(Stdio::null()).spawn() {
        Ok(c) => c, Err(_) => return Ok(false),
    };
    child.stdin.as_mut().unwrap().write_all(text.as_bytes())?;
    Ok(child.wait()?.success())
}
```

**Tests** (`crates/daemon/src/ipc.rs`):
- Cannot easily unit-test IPC. Add ONE integration-style test in `crates/daemon/tests/apply_test.rs` (NEW): boot daemon with `tempfile::TempDir` for `ORGANISM_HOME`, seed `error_<h>.json` + `suggestion_<h>.json` (suggestion contains a diff block), connect to socket, send `apply` request with `mode: "dry"`, assert response `plan_kind == "patch"` and message contains `"-a"`.

**Acceptance:** `cargo test -p organism-daemon apply` green. `cargo clippy -p organism-daemon -- -D warnings` clean.

---

### TASK-L3.5-05 — CLI `apply` command

**File:** `crates/client/src/main.rs`. Add subcommand mirroring existing `suggest`:

```
organism-cli apply <ERROR_KEY> [--stage]
```

- `<ERROR_KEY>` required (8-16 hex chars). If user omits, error out with hint to run `organism-cli log` or `suggest` first.
- `--stage` flag → `ApplyMode::Stage`. Default `Dry`.
- Send `apply` IPC envelope. Print response `message` to stdout. If `artifact_path` set, also print `artifact: <path>` on a separate line.

**Tests:** integration test in `crates/client/tests/apply_cli_test.rs` (NEW) — spawn daemon (reuse pattern from existing `suggest_cli_test.rs` if present, else stub a fake socket server with `tokio::net::UnixListener` returning canned `ApplyResponse`).

**Acceptance:** `cargo test -p organism-client` green.

---

### TASK-L3.5-06 — README + LEARNINGS

**File:** `README.md`. Under "Usage", append:

````markdown
### Apply a suggestion (L3.5)

```bash
# preview only — prints plan, does nothing
organism-cli apply <error-hash>

# stage it: writes diff to /tmp or copies shell cmd to clipboard
organism-cli apply <error-hash> --stage
```

Daemon never writes to your source files. `--stage` produces an artifact (patch
file or clipboard copy) that you apply yourself.
````

Update Status matrix: add row `L3.5 Effector seed | apply CLI, dry/stage modes, no auto-mutation | DONE`.

**File:** `LEARNINGS.md`. Append entry per task with date `2026-04-30`.

**Acceptance:** files updated; no other code changes.

---

## Dependency DAG

```
01 (parser) ──┐
              ├──> 04 (daemon handler) ──> 05 (CLI) ──> 06 (docs)
02 (proto) ───┤
03 (kv pair) ─┘
```

01, 02, 03 parallelizable. 04 needs all three. 05 needs 04. 06 last.

---

## Acceptance gate (whole milestone)

1. `cargo build --workspace --locked` green.
2. `cargo test --workspace --locked` green (incl. new apply tests).
3. `cargo clippy --workspace -- -D warnings` clean.
4. `cargo fmt --all -- --check` clean.
5. **Manual smoke** on real `~/.organism/`:
   - `organism-cli suggest` (cache a suggestion).
   - `organism-cli apply <hash>` → prints plan.
   - `organism-cli apply <hash> --stage` → either `/tmp/organism-<hash>.patch` exists OR `pbpaste` returns the shell cmd.
6. Daemon does NOT modify any file under user's repo. Verify by `git status` clean before/after.
7. PR opened, CI green (4 contexts: fmt, build-and-test ubuntu, build-and-test macos, clippy), squash-merge to `main`.
8. Tag `v0.3.0`, push, release workflow uploads `organism-v0.3.0-{darwin-arm64,linux-x86_64}.tar.gz`.

---

## Pitfalls pre-flagged

- **Regex multiline**: use `(?s)` flag so `.` matches newlines inside fenced blocks.
- **No clipboard on Linux CI**: `xclip` not installed in `ubuntu-latest`; `copy()` returns `Ok(false)` and handler falls back to printing — test must assert this fallback path on Linux.
- **`/tmp` perms**: use `tempfile::NamedTempFile` for tests, not raw `/tmp` writes (CI workspace may differ).
- **Suggestion with no fenced block**: most LLM outputs will hit `Note` path. That's fine — confirms parser doesn't false-match prose.
- **Concurrent apply**: don't worry. CLI is one-shot; daemon handles serially per-connection.
- **Path traversal**: `error_key` must be `[a-f0-9]{1,64}`. Validate in CLI AND in daemon handler before opening files. Reject otherwise.
- **Don't shell out to `git apply` from daemon** — that's L4 effector territory. v1 only writes the patch file; user runs `git apply` themselves.

---

## Files touched (summary)

| Action | Path |
|--------|------|
| NEW | `crates/cortex/src/apply.rs` |
| EDIT | `crates/cortex/src/lib.rs` (`pub mod apply;`) |
| EDIT | `crates/cortex/Cargo.toml` (regex dep if missing) |
| EDIT | `crates/protocol/src/messages.rs` (Apply types) |
| EDIT | `crates/knowledge/src/store.rs` (`load_pair`) |
| EDIT | `crates/daemon/src/ipc.rs` (apply handler) |
| NEW | `crates/daemon/src/clipboard.rs` |
| EDIT | `crates/daemon/src/lib.rs` or `main.rs` (`mod clipboard;`) |
| NEW | `crates/daemon/tests/apply_test.rs` |
| EDIT | `crates/client/src/main.rs` (apply subcommand) |
| NEW | `crates/client/tests/apply_cli_test.rs` |
| EDIT | `README.md` (usage + status) |
| EDIT | `LEARNINGS.md` (per-task) |

---

## Estimated effort

- Junior dev + small local LLM: **6–10 hours** total. Each task ≤90min.
- Order: 01 → 02 → 03 (parallel-ish) → 04 → 05 → 06.
- Local LLM strength: tasks 01, 02, 03, 05 are mechanical (mirror existing patterns). Task 04 is the one that needs careful reading of `ipc.rs::dispatch`.
