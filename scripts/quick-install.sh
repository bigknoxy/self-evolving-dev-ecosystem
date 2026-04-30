#!/usr/bin/env bash
# quick-install.sh -- one-line installer. Clones repo + runs scripts/install.sh.
set -euo pipefail

REPO="bigknoxy/self-evolving-dev-ecosystem"
BRANCH="${ORGANISM_BRANCH:-main}"

say() { echo "==> $*"; }
die() { echo "error: $*" >&2; exit 1; }

case "$(uname -s)" in
  Darwin|Linux) ;;
  *) die "unsupported platform: $(uname -s) -- macOS or Linux only" ;;
esac

command -v git >/dev/null || die "git not found on PATH -- install git first"
command -v cargo >/dev/null || die "cargo not found on PATH -- install Rust toolchain: https://rustup.rs"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

say "Cloning ${REPO}@${BRANCH} into ${TMP}"
git clone --depth 1 --branch "${BRANCH}" "https://github.com/${REPO}.git" "${TMP}/organism"

say "Running scripts/install.sh"
cd "${TMP}/organism"
bash scripts/install.sh

cat <<EOF

==> Quick install complete.
    Open a new shell or 'source ~/.zshrc' to activate the hook.
    Try: organism-cli status
EOF
