# L2 FINAL ACCEPTANCE — End-to-End Smoke

## Goal
Prove install + runtime path: shell hook → daemon IPC → file watcher → terminal sensor → error classifier → knowledge store.

## Pre-flight (verify, do not assume)
- `cargo build --workspace --release` succeeds
- `target/release/organism-daemon` and `target/release/organism-cli` exist
- No daemon currently running (`pgrep -f organism-daemon` empty); kill if found
- Use ISOLATED `ORGANISM_HOME=/tmp/organism-smoke-<ts>` — do NOT touch real `~/.organism/`
- Use ISOLATED `~/.local/bin/` IS real but we install only into a tmp prefix → modify install.sh invocation via `PREFIX` env or run scripts manually with overridden paths. If install.sh hardcodes `~/.local/bin`, run it with `HOME=/tmp/organism-smoke-home-<ts>` to sandbox.
- Do NOT modify real `~/.zshrc`. Do NOT load real LaunchAgent.

## Wave A: Real install (sandboxed)
1. `mkdir -p /tmp/orgsmoke/home && export HOME=/tmp/orgsmoke/home`
2. Run `bash scripts/install.sh` — captures real cargo build + copy + zshrc append + plist install (under sandboxed HOME, launchctl load may fail in non-GUI session — that's OK, capture warning)
3. Verify:
   - `/tmp/orgsmoke/home/.local/bin/organism-daemon` exists, executable
   - `/tmp/orgsmoke/home/.local/bin/organism-cli` exists, executable
   - `/tmp/orgsmoke/home/.zshrc` contains marker block
   - `/tmp/orgsmoke/home/Library/LaunchAgents/com.organism.daemon.plist` exists, `__HOME__` substituted, `plutil -lint` ok
4. Idempotency: run install.sh again, assert no duplicate marker block in zshrc (count == 1)

## Wave B: Runtime smoke
1. Start daemon: `ORGANISM_HOME=/tmp/orgsmoke/data /tmp/orgsmoke/home/.local/bin/organism-daemon &` (background)
2. Wait ≤3s for socket: poll `/tmp/orgsmoke/data/daemon.sock` exists
3. `organism-cli status` → asserts `awake: true`, `event_count: 0`
4. **File watcher path:** `touch /tmp/orgsmoke/data/test.txt` → wait 500ms → `organism-cli log` shows File event for test.txt
   - Note: file watcher watches `current_dir` of daemon; cd into a tmp watch dir before launch, OR set watch root via env. Verify behavior via daemon source.
5. **Terminal + classifier path:** `organism-cli emit-terminal --command "cargo build" --exit-code 101 --stderr-snippet "error[E0599]: no method named foo"`
   - `organism-cli log` shows Terminal event with exit_code=101
   - Inspect knowledge store directly: `ls /tmp/orgsmoke/data/knowledge/` — assert at least one `error_*` key exists with tool=rustc kind=E0599
6. **Increment:** repeat emit-terminal same args, assert occurrences == 2
7. **Sleep gate:** `organism-cli sleep` → emit-terminal → log unchanged (no new event)
8. **Wake:** `organism-cli wake` → emit-terminal → new event appears

## Wave C: Cleanup
1. Stop daemon (kill PID)
2. `bash scripts/uninstall.sh` (under sandboxed HOME)
3. Verify binaries gone, marker stripped from zshrc, plist removed
4. `rm -rf /tmp/orgsmoke`

## Coverage gate
- Every assertion above MUST execute and report pass/fail
- If watcher root behavior unclear, READ `crates/daemon/src/main.rs` to confirm before testing
- Report includes: which exact daemon flag/env controls watch root, ANY behavior surprises

## Out of scope
- Loading real LaunchAgent into real launchctl (skip; only verify plist file written + plutil-lint)
- Modifying real ~/.zshrc

## Pitfalls pre-flagged
- Daemon may use `current_dir` not configurable env → cd before launch
- launchctl load fails outside aqua session → OK, log only
- macOS `~/.local/bin` not on PATH by default → use absolute paths in smoke tests
- `organism-cli` needs ORGANISM_HOME too — set on every call to find socket
