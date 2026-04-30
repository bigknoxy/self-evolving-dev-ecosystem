#!/usr/bin/env bash
# quick-uninstall.sh -- one-line uninstaller. Clones repo + runs scripts/uninstall.sh.
set -euo pipefail

REPO="bigknoxy/self-evolving-dev-ecosystem"
BRANCH="${ORGANISM_BRANCH:-main}"

say() { echo "==> $*"; }
die() { echo "error: $*" >&2; exit 1; }

command -v git >/dev/null || die "git not found on PATH -- install git first"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

say "Cloning ${REPO}@${BRANCH} into ${TMP}"
git clone --depth 1 --branch "${BRANCH}" "https://github.com/${REPO}.git" "${TMP}/organism"

say "Running scripts/uninstall.sh"
cd "${TMP}/organism"
bash scripts/uninstall.sh

cat <<EOF

==> Quick uninstall complete.
    Note: ~/.organism/ data dir preserved. Remove with: rm -rf ~/.organism
EOF
