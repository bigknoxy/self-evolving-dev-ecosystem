# Self-Evolving Dev Ecosystem (Organism)

Learns from your dev failures and errors without leaving your machine. Daemon watches
your terminal and files, classifies what goes wrong, builds a personal knowledge store,
and over time can suggest fixes — no cloud, no telemetry.

## What it is

Local Rust daemon that watches your dev activity — terminal commands, file
changes — classifies failures, and writes them to a personal knowledge store
under `~/.organism/`. Self-evolving because the dataset is the substrate for
later layers: L3 plugs in Ollama for suggestions, L4 grows it into a digital
twin. Local-first, no network.

## Quick Start

1. **Install** (requires `git` and `cargo` on PATH):
   ```bash
   curl -fsSL https://raw.githubusercontent.com/bigknoxy/self-evolving-dev-ecosystem/main/scripts/quick-install.sh | bash
   ```

2. **Open a new shell** so the zsh hook and PATH setup take effect:
   ```bash
   exec zsh
   ```

3. **Verify the daemon is running**:
   ```bash
   organism-cli status
   ```

4. **Generate an error event** by running a command that fails (e.g., in a broken project):
   ```bash
   cargo build
   ```
   The daemon catches the error and stores it.

5. **Get a suggestion** (requires `ollama serve` running and `OLLAMA_ENABLED=1`):
   ```bash
   OLLAMA_ENABLED=1 organism-cli suggest
   ```

## Architecture

5-crate Cargo workspace.

| Crate | Role |
|-------|------|
| `organism-protocol` | Event/envelope types, IPC message schema (serde) |
| `organism-knowledge` | File-backed KV store under `$ORGANISM_HOME` |
| `organism-cortex` | Pattern engine + error classifier (rustc/npm/python/shell regex) |
| `organism-daemon` | Bin `organism`. Event bus, IPC server, file + terminal sensors, error subscriber |
| `organism-client` | Bin `organism-cli`. Talks to the daemon over Unix socket |

- IPC: Unix domain socket at `$ORGANISM_HOME/daemon.sock`, newline-delimited JSON envelopes, one request per connection.
- Bus: `tokio::sync::broadcast`. Producers (sensors, IPC) record events; subscribers (error classifier) react.

## Status

| Level | Scope | State |
|-------|-------|-------|
| L0 Observer | Event bus, knowledge store, pattern engine, CLI skeleton | DONE |
| L1 Sensor wiring | Bidirectional Unix socket IPC, zsh hook → `emit-terminal` | DONE |
| L2 Watcher + classifier + install | `notify` file watcher, regex error classifier, `install.sh` + LaunchAgent | DONE |
| L3 Ollama integration | Ollama HTTP client, `suggest` module, daemon subscriber, CLI `suggest` command (gated by `OLLAMA_ENABLED=1`) | DONE |
| L4 Digital twin | Codes alongside you in your style | PLANNED |

## Install

### Quick (one-line)

```bash
# install
curl -fsSL https://raw.githubusercontent.com/bigknoxy/self-evolving-dev-ecosystem/main/scripts/quick-install.sh | bash

# uninstall
curl -fsSL https://raw.githubusercontent.com/bigknoxy/self-evolving-dev-ecosystem/main/scripts/quick-uninstall.sh | bash
```

Requires `git` + `cargo` on PATH. macOS or Linux only.

### Manual

```bash
bash scripts/install.sh --dry-run   # preview
bash scripts/install.sh             # for real
```

What it does:

- `cargo build --workspace --release`
- copies `target/release/organism` → `~/.local/bin/organism-daemon`
- copies `target/release/organism-cli` → `~/.local/bin/organism-cli`
- copies `scripts/organism-shell-hook.zsh` → `~/.organism/shell/zsh-hook.sh`
- appends a marked block to `~/.zshrc` that puts `~/.local/bin` on PATH and sources the hook (idempotent — guarded by marker)
- on macOS, writes `~/Library/LaunchAgents/com.organism.daemon.plist` and `launchctl load`s it (best-effort; warns and continues in non-GUI sessions)

Caveats:

- Shell hook is zsh-only.
- LaunchAgent step is macOS-only. Daemon itself runs on Linux; you supply your own service unit.

## Usage

```bash
# daemon status
organism-cli status

# recent events ring buffer
organism-cli log

# pause / resume event recording
organism-cli sleep
organism-cli wake

# manually inject a terminal event (this is what the zsh hook calls)
organism-cli emit-terminal "cargo build" \
  --exit-code 101 \
  --cwd /path/to/proj \
  --duration-ms 1820 \
  --stderr "error[E0599]: no method named foo"

# context-aware suggestion (requires L3 + Ollama)
OLLAMA_ENABLED=1 organism-cli suggest
```

Suggestion environment variables:

- `OLLAMA_ENABLED` — set to `1` to enable suggestions (default: `0` / disabled)
- `OLLAMA_BASE_URL` — Ollama HTTP endpoint (default: `http://127.0.0.1:11434`)
- `OLLAMA_MODEL` — model to use (default: `qwen2.5-coder:7b`)

Notes:

- File watcher auto-roots at the daemon's launch `cwd`; recursive; ignores `target/`, `.git/`, dotfiles.
- Failed terminal events (`exit_code != 0`) are classified by `organism-cortex` and persisted as `ErrorRecord`s; duplicate signatures bump `occurrences`.
- Knowledge store layout: flat JSON files under `$ORGANISM_HOME/knowledge/`, e.g. `error_<hash>.json`, `pattern_<hash>.json`. Override base with `ORGANISM_HOME` env var (default `~/.organism`).

## Who it's for

Solo devs who want their tooling to learn from their friction. Not a team
product. No telemetry, no cloud. macOS-first install path; daemon runs
anywhere Tokio does.

Not for you if:
- Your team needs shared error/pattern data across developers
- You want to sync knowledge to the cloud
- You use Windows (daemon runs on Linux/macOS only)

## Uninstall

```bash
bash scripts/uninstall.sh
```

Removes binaries, hook copy, LaunchAgent, and the marked zshrc block. Leaves
`~/.organism/` data dir intact — `rm -rf ~/.organism` if you want it gone.

## Dev

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Build order matters: `protocol` and `knowledge` before `cortex` before
`daemon`. See `CLAUDE.md` and `AGENTS.md` for contributor standards
(error handling, serde requirements, test conventions, clippy policy).

Architecture detail in `IMPLEMENTATION.md`. Task history in `TASKS.md`.
Per-task notes and gotchas in `LEARNINGS.md`.

## Roadmap

L4+ scope (deliberately out of scope for L0–L3):

- Real digital-twin code generation in user style
- Inline suggestion UI / editor surface
- Effector framework — daemon takes actions (format, patch, scan), not just observes
- Windows + Linux service installers (LaunchAgent today is macOS-only)
- Plugin API for project-specific sensors/effectors (React, Python, etc.)
