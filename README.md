# Self-Evolving Dev Ecosystem (Organism)

A local Rust daemon that watches your terminal, classifies your dev failures, and
learns from every fix you accept or apply — no cloud, no telemetry, no subscription.

---

## Why this exists

Every time you hit a repeated build error and fix it, that knowledge evaporates.
Next week, same error, same fifteen minutes of debugging. Organism remembers.

The daemon watches your terminal in the background. When `cargo build` fails, it
classifies the error, stores it, and — if you have Ollama running — generates a
suggestion. When you accept or apply a fix, the daemon captures that signal and
refines its understanding of your preferences: which tools you trust, how verbose
you like explanations, what kind of patches you actually reach for.

Over time the knowledge store becomes a personal record of your friction: what
breaks, what fixes it, what you tried and skipped. That record is the substrate
for everything above L3.

**Local-first.** The binary runs on your machine. Knowledge lives in `~/.organism/`.
Nothing leaves your host.

---

## Who should use this

**Good fit:**
- Solo developers who work heavily with `cargo`, `npm`, `python`, or shell tools
  and see the same errors repeat across projects
- Developers who want local AI assistance without API keys or cloud accounts
- Anyone who installs Ollama and wants it to actually know their development context

**Not for you if:**
- Your team needs shared error/pattern data across developers
- You want cloud sync or cross-machine knowledge
- You use Windows (daemon requires macOS or Linux)

---

## How it works

```
terminal (zsh hook) ─────┐
file changes (notify) ────┤──→ EventBus ──→ ErrorClassifier ──→ KnowledgeStore
manual emit (IPC) ────────┘                      │
                                                  ↓
                                        OllamaSubscriber → suggestion cache
                                                  │
                                        organism-cli suggest / apply / feedback
                                                  │
                                        StyleProfile (learns your preferences)
```

1. **zsh hook** fires on every command exit. If exit code ≠ 0, daemon sees the error.
2. **ErrorClassifier** extracts a signature hash (`tool:kind:hash`). Duplicate errors
   bump `occurrences` instead of creating new records.
3. **Ollama subscriber** (when `OLLAMA_ENABLED=1`) generates and caches a suggestion
   for each unique error. Fires a desktop notification when the tool's accept rate
   exceeds the notification gate (≥70%).
4. **CLI** lets you see suggestions, apply them (dry-run or staged), and record what
   you did with them (`accept`, `reject`, `ignore`, `applied`).
5. **StyleProfile** is rebuilt from your feedback history: accept rates by tool,
   preferred verbosity, common phrases in accepted suggestions. Future prompts use
   this profile to match your style.

---

## Real use cases

**Repeated build error → auto-suggestion**
```
$ cargo build
error[E0599]: no method named `bar` on type `Foo`

$ organism-cli suggest
[cached] Try implementing `bar` on `Foo` or use `.baz()` instead.
Occurred 3×. Confidence: high.

Did you apply this patch? [y/N]: y
# daemon records Verdict::Applied — 2× weight in style profile
```

**Learning your tool preferences**
After 20 feedback events:
```
$ organism-cli doctor
daemon:   running (pid 41823)
notification gates:
  cargo                accept_rate=0.82  [notifiable ≥0.70]
  npm                  accept_rate=0.43  [silent <0.70]
  python               accept_rate=0.61  [silent <0.70]
```
Notifications only fire for tools where you've historically accepted fixes.

**Stats across sessions**
```
$ organism-cli stats
Metrics
  since: 2026-04-01T00:00:00Z
  prompt version: m17-apply-v1

Current:
  suggestions total: 47
  suggestions cached: 31
  feedback: 2 applied, 18 accepted, 6 rejected
  acceptance: 20/26 = 76.9%

By tool:
  cargo: 15 accepts, 3 rejects (83.3%)
  npm:   3 accepts, 3 rejects (50.0%)
```

**Baseline tracking (before/after a prompt change)**
```
$ organism-cli stats --capture-baseline
baseline captured at 2026-05-15T12:00:00Z

# ... use the tool for a week ...

$ organism-cli stats --baseline
Delta vs baseline:
  feedback: 1 applied, 4 accepted, 1 rejected
  acceptance: 5/6 = 83.3%
```

---

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

4. **Generate an error event** by running a command that fails:
   ```bash
   cargo build    # in any project with a compile error
   ```
   The daemon classifies the error and stores it.

5. **Get a suggestion** (requires `ollama serve` + `OLLAMA_ENABLED=1`):
   ```bash
   OLLAMA_ENABLED=1 organism-cli suggest
   ```

---

## Status

| Level | Scope | State |
|-------|-------|-------|
| L0 Observer | Event bus, knowledge store, pattern engine, CLI skeleton | DONE |
| L1 Sensor wiring | Bidirectional Unix socket IPC, zsh hook → `emit-terminal` | DONE |
| L2 Watcher + classifier + install | `notify` file watcher, regex error classifier, `install.sh` + LaunchAgent | DONE |
| L3 Ollama integration | Ollama HTTP client, `suggest` module, daemon subscriber, CLI `suggest` | DONE |
| L3.5 Effector seed | `apply` IPC + CLI: patch/shell/note plans; `--stage` writes patch or copies shell | DONE |
| M6–M17 Style + feedback loop | Feedback capture, StyleProfile, few-shot prompts, apply-outcome prompt, notification gates, metrics/baseline, PII redaction | DONE |
| L4 Digital twin | Codes alongside you in your style | PLANNED |

### Feature highlights (M6–M17)

| Milestone | What shipped |
|-----------|-------------|
| M6 | `organism-cli feedback` — accept/reject/ignore a suggestion |
| M7 | Multi-block plan parsing — suggestions can contain patch + shell + note |
| M8 | PII redaction (emails, tokens, UUIDs) + remote-URL gate |
| M9 | Schema versioning + `organism-cli doctor` |
| M9.5 | Immutable accepted-suggestion snapshots (training signal preservation) |
| M10 | StyleProfile — phrase mining, IPC, `organism-cli style` |
| M11 | Few-shot context injection — kNN over accepted suggestions in Ollama prompt |
| M13 | Proactive desktop notifications gated on per-tool accept rate |
| M15 | `organism-cli stats` — metrics + baseline capture |
| M16 | Gate status in `organism-cli doctor` |
| M17 | Post-apply prompt + `Verdict::Applied` (2× weight), stats breakdown |

---

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
- appends a marked block to `~/.zshrc` that puts `~/.local/bin` on PATH and sources the hook (idempotent)
- on macOS, writes `~/Library/LaunchAgents/com.organism.daemon.plist` and `launchctl load`s it

Caveats:

- Shell hook is zsh-only.
- LaunchAgent step is macOS-only. Daemon itself runs on Linux; you supply your own service unit.

### Environment Overrides (`~/.organism/env`)

The LaunchAgent plist is generated with default environment variables. Create `~/.organism/env`
with `KEY=VAL` entries (one per line) before running `install.sh` to customize them.

**Default variables:**
- `OLLAMA_ENABLED=1` — enable Ollama-based suggestions
- `OLLAMA_BASE_URL=http://127.0.0.1:11434` — local Ollama server
- `OLLAMA_MODEL=qwen2.5-coder:7b` — default model for suggestions
- `ORGANISM_HOME=$HOME/.organism` — knowledge store location
- `PATH=$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin` — daemon PATH
- `RUST_LOG=info` — log level

After editing `~/.organism/env`, rerun `bash scripts/install.sh` to regenerate the plist.
Invalid key names (not matching `^[A-Z_][A-Z0-9_]*$`) are skipped with a warning.

---

## Usage

```bash
# daemon status
organism-cli status

# recent events ring buffer
organism-cli log

# list classified errors (most recent first)
organism-cli errors

# pause / resume event recording
organism-cli sleep
organism-cli wake

# context-aware suggestion (requires Ollama)
OLLAMA_ENABLED=1 organism-cli suggest

# apply a suggestion (dry-run by default)
organism-cli apply <error-hash>

# stage a suggestion: writes patch to /tmp or copies shell cmd to clipboard
# prompts "Did you apply this patch?" when stdin is a terminal
organism-cli apply <error-hash> --stage

# record your verdict on a suggestion
organism-cli feedback <error-hash> accept   # or reject / ignore / applied

# style profile (built from your feedback history)
organism-cli style

# metrics (suggestions, feedback counts, acceptance rate by tool)
organism-cli stats
organism-cli stats --json

# baseline workflow
organism-cli stats --capture-baseline
# ...use the tool for a week...
organism-cli stats --baseline          # shows delta vs baseline

# health check + notification gate status per tool
organism-cli doctor

# manually inject a terminal event (what the zsh hook calls)
organism-cli emit-terminal "cargo build" \
  --exit-code 101 \
  --cwd /path/to/proj \
  --duration-ms 1820 \
  --stderr "error[E0599]: no method named foo"
```

### Apply workflow

```bash
# 1. See what failed
organism-cli errors

# 2. Preview the suggestion plan
organism-cli apply <hash>

# 3. Stage it — writes patch to /tmp or clipboard
organism-cli apply <hash> --stage
# → "Did you apply this patch? [y/N]:" appears when stdin is a terminal
# → answering y records Verdict::Applied (2× signal weight)
```

`<error-hash>` is the hex hash shown by `organism-cli errors`. Daemon never writes
to your source files. `--stage` produces an artifact you apply yourself.

### Notification gate

The daemon fires a desktop notification when an error recurs AND the tool's accept
rate in your StyleProfile is ≥70%. Use `organism-cli doctor` to see which tools
are above the gate. Use `organism-cli feedback` to build the profile faster.

---

## Knowledge store layout

Flat JSON files under `$ORGANISM_HOME/knowledge/` (default `~/.organism/knowledge/`):

```
error_<hash>.json           — ErrorRecord (tool, kind, hash, occurrences, …)
suggestion_<hash>.json      — SuggestionRecord (cached LLM output)
accepted_<hash>.json        — AcceptedSuggestion (immutable snapshot at accept time)
feedback_<hash>.json        — FeedbackRecord (verdict, timestamp)
pattern_<hash>.json         — PatternRecord (learned trigger/action pairs)
style_profile_current.json  — StyleProfile (accept rates, phrases, terseness)
```

Override the base with `ORGANISM_HOME` env var.

---

## Uninstall

```bash
bash scripts/uninstall.sh
```

Removes binaries, hook copy, LaunchAgent, and the marked zshrc block. Leaves
`~/.organism/` data dir intact — `rm -rf ~/.organism` if you want it gone.

---

## Dev

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Build order matters: `protocol` and `knowledge` before `cortex` before
`daemon`. See `CLAUDE.md` and `AGENTS.md` for contributor standards
(error handling, serde requirements, test conventions, clippy policy).

Architecture detail in `IMPLEMENTATION.md`. Per-task notes and gotchas in `LEARNINGS.md`.

---

## Roadmap

L4+ scope (deliberately out of scope for L0–M17):

- Real digital-twin code generation in user style (uses StyleProfile + few-shot history)
- Inline suggestion UI / editor surface
- Effector framework — daemon takes actions (format, patch, scan), not just observes
- `organism-cli export` — portable snapshot of errors + suggestions + feedback
- Windows + Linux service installers (LaunchAgent today is macOS-only)
- Plugin API for project-specific sensors/effectors (React, Python, etc.)
