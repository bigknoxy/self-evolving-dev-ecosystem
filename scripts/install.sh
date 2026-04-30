#!/usr/bin/env bash
# install.sh -- install the organism daemon, CLI, shell hook, and macOS LaunchAgent.
set -euo pipefail

DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    -h|--help)
      echo "Usage: $0 [--dry-run]"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK_SRC="${REPO_ROOT}/scripts/organism-shell-hook.zsh"
PLIST_SRC="${REPO_ROOT}/scripts/com.organism.daemon.plist"

BIN_DIR="${HOME}/.local/bin"
ORG_HOME="${HOME}/.organism"
HOOK_DST_DIR="${ORG_HOME}/shell"
HOOK_DST="${HOOK_DST_DIR}/zsh-hook.sh"
LAUNCH_AGENTS_DIR="${HOME}/Library/LaunchAgents"
PLIST_DST="${LAUNCH_AGENTS_DIR}/com.organism.daemon.plist"
ZSHRC="${HOME}/.zshrc"
MARKER_BEGIN="# >>> organism shell hook >>>"
MARKER_END="# <<< organism shell hook <<<"

run() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[dry-run] $*"
  else
    echo "+ $*"
    eval "$@"
  fi
}

say() { echo "==> $*"; }

# 1. Build release binaries.
say "Building release binaries"
run "cd '${REPO_ROOT}' && cargo build --workspace --release"

# 2. Install binaries.
# Note: the daemon crate's [[bin]] name is "organism" (see crates/daemon/Cargo.toml),
# so the build artifact is target/release/organism. We rename it to organism-daemon
# on copy so the installed name matches the LaunchAgent plist + user expectations.
say "Installing binaries to ${BIN_DIR}"
run "mkdir -p '${BIN_DIR}'"
DAEMON_SRC="${REPO_ROOT}/target/release/organism"
CLI_SRC="${REPO_ROOT}/target/release/organism-cli"
if [[ "$DRY_RUN" -ne 1 ]]; then
  if [[ ! -f "${DAEMON_SRC}" ]]; then
    echo "Error: daemon binary not found at ${DAEMON_SRC}" >&2
    echo "       (the daemon crate's bin name is 'organism' -- did 'cargo build --release' succeed?)" >&2
    exit 1
  fi
  if [[ ! -f "${CLI_SRC}" ]]; then
    echo "Error: CLI binary not found at ${CLI_SRC}" >&2
    exit 1
  fi
fi
run "cp '${DAEMON_SRC}' '${BIN_DIR}/organism-daemon'"
run "cp '${CLI_SRC}' '${BIN_DIR}/organism-cli'"

# 3. Install shell hook + idempotent zshrc append.
say "Installing zsh hook to ${HOOK_DST}"
run "mkdir -p '${HOOK_DST_DIR}'"
run "cp '${HOOK_SRC}' '${HOOK_DST}'"

if [[ -f "${ZSHRC}" ]] && grep -Fq "${MARKER_BEGIN}" "${ZSHRC}"; then
  say "zshrc already contains organism hook marker -- skipping append"
else
  say "Appending organism hook block to ${ZSHRC}"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[dry-run] append organism hook block to ${ZSHRC}"
  else
    {
      printf '\n%s\n' "${MARKER_BEGIN}"
      printf 'export PATH="%s:$PATH"\n' "${BIN_DIR}"
      printf '[ -f "%s" ] && source "%s"\n' "${HOOK_DST}" "${HOOK_DST}"
      printf '%s\n' "${MARKER_END}"
    } >> "${ZSHRC}"
  fi
fi

# 4. macOS LaunchAgent.
if [[ "$(uname)" == "Darwin" ]]; then
  say "Installing LaunchAgent to ${PLIST_DST}"
  run "mkdir -p '${LAUNCH_AGENTS_DIR}'"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[dry-run] sed s|__HOME__|${HOME}|g '${PLIST_SRC}' > '${PLIST_DST}'"
  else
    sed "s|__HOME__|${HOME}|g" "${PLIST_SRC}" > "${PLIST_DST}"
  fi
  run "launchctl unload '${PLIST_DST}' 2>/dev/null || true"
  # Guard launchctl load: it can fail in non-GUI sessions (sandboxed HOME, ssh w/o
  # GUI auth context, CI). Don't abort the whole install -- plist is in place and
  # will be loaded on next login.
  run "launchctl load '${PLIST_DST}' || echo 'warn: launchctl load failed (non-GUI session?) -- plist installed but not loaded'"
else
  say "Non-Darwin platform detected -- skipping LaunchAgent install"
fi

# 5. Ensure organism home dir exists.
say "Ensuring ${ORG_HOME} exists"
run "mkdir -p '${ORG_HOME}'"

cat <<EOF

==> Install complete$( [[ ${DRY_RUN} -eq 1 ]] && echo " (dry-run)" )
    binaries: ${BIN_DIR}/organism-{daemon,cli}
    shell hook: ${HOOK_DST}
    zshrc:    ${ZSHRC} (sources hook between markers)
    plist:    ${PLIST_DST} (Darwin only)
    home:     ${ORG_HOME}

Open a new shell or 'source ${ZSHRC}' to activate the hook.
EOF
