#!/usr/bin/env bash
# uninstall.sh -- reverse install.sh.
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

BIN_DIR="${HOME}/.local/bin"
ORG_HOME="${HOME}/.organism"
HOOK_DST="${ORG_HOME}/shell/zsh-hook.sh"
PLIST_DST="${HOME}/Library/LaunchAgents/com.organism.daemon.plist"
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

# 1. macOS LaunchAgent teardown.
if [[ "$(uname)" == "Darwin" ]]; then
  say "Tearing down LaunchAgent"
  if [[ -f "${PLIST_DST}" ]] || [[ "$DRY_RUN" -eq 1 ]]; then
    run "launchctl unload '${PLIST_DST}' 2>/dev/null || true"
    run "rm -f '${PLIST_DST}'"
  else
    say "no plist at ${PLIST_DST}"
  fi
fi

# 2. Remove binaries.
say "Removing binaries from ${BIN_DIR}"
run "rm -f '${BIN_DIR}/organism-daemon'"
run "rm -f '${BIN_DIR}/organism-cli'"

# 3. Remove shell hook copy.
say "Removing installed shell hook"
run "rm -f '${HOOK_DST}'"

# 4. Strip marker block from zshrc.
if [[ -f "${ZSHRC}" ]]; then
  if grep -Fq "${MARKER_BEGIN}" "${ZSHRC}"; then
    say "Stripping organism hook block from ${ZSHRC} (backup: ${ZSHRC}.bak)"
    if [[ "$DRY_RUN" -eq 1 ]]; then
      echo "[dry-run] sed -i.bak '/${MARKER_BEGIN}/,/${MARKER_END}/d' '${ZSHRC}'"
    else
      sed -i.bak "/${MARKER_BEGIN}/,/${MARKER_END}/d" "${ZSHRC}"
    fi
  else
    say "No organism marker found in ${ZSHRC} -- nothing to strip"
  fi
else
  say "${ZSHRC} not present -- skipping"
fi

cat <<EOF

==> Uninstall complete$( [[ ${DRY_RUN} -eq 1 ]] && echo " (dry-run)" )
    Note: ${ORG_HOME} (knowledge store + logs) preserved.
          Remove manually with: rm -rf ${ORG_HOME}
EOF
