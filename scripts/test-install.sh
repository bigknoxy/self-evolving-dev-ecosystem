#!/usr/bin/env bash
# test-install.sh -- M5 plist EnvironmentVariables install test (macOS only)
set -euo pipefail

# macOS-only guard
if [[ "$(uname)" != "Darwin" ]]; then
    echo "skip: macOS only"
    exit 0
fi

echo "==> M5 install sandbox test starting..."

# Sandbox: create temp HOME
TEMP_HOME=$(mktemp -d)
trap "rm -rf '$TEMP_HOME'" EXIT

export HOME="$TEMP_HOME"
echo "    Sandbox HOME: $HOME"

# Create required directories
mkdir -p "$HOME/Library/LaunchAgents"
mkdir -p "$HOME/.organism"

# Create ~/.organism/env with test override
cat > "$HOME/.organism/env" <<'ENV_EOF'
# Test override
OLLAMA_MODEL=llama3:8b
TEST_CUSTOM_VAR=custom_value
ENV_EOF

echo "    Environment override file created"
echo "    Contents:"
cat "$HOME/.organism/env" | sed 's/^/      /'

# Run install script with ORGANISM_SKIP_BUILD to avoid 5min build
echo ""
echo "==> Running install script..."
ORGANISM_SKIP_BUILD=1 bash scripts/install.sh

PLIST="$HOME/Library/LaunchAgents/com.organism.daemon.plist"

if [[ ! -f "$PLIST" ]]; then
    echo "ERROR: plist not created at $PLIST" >&2
    exit 1
fi

echo ""
echo "==> Running assertions..."

# 1. plutil lint — validate XML structure
echo "    [1/5] Checking plist validity with plutil..."
if plutil -lint "$PLIST" >/dev/null 2>&1; then
    echo "         ✓ plist lint passed"
else
    echo "         ✗ plist lint FAILED" >&2
    plutil -lint "$PLIST" || true
    exit 1
fi

# 2. grep for EnvironmentVariables section
echo "    [2/5] Checking EnvironmentVariables section..."
if grep -q "EnvironmentVariables" "$PLIST"; then
    echo "         ✓ EnvironmentVariables section found"
else
    echo "         ✗ EnvironmentVariables section missing" >&2
    exit 1
fi

# 3. grep for default env var (OLLAMA_ENABLED)
echo "    [3/5] Checking default env vars..."
if grep -q "OLLAMA_ENABLED" "$PLIST"; then
    echo "         ✓ Default OLLAMA_ENABLED found"
else
    echo "         ✗ Default OLLAMA_ENABLED missing" >&2
    exit 1
fi

# 4. grep that __HOME__ was substituted
echo "    [4/5] Checking __HOME__ substitution..."
if grep -q "__HOME__" "$PLIST"; then
    echo "         ✗ __HOME__ placeholder NOT substituted (still in plist)" >&2
    exit 1
else
    if grep -q "$HOME" "$PLIST"; then
        echo "         ✓ __HOME__ substituted to actual path"
    else
        echo "         ✗ __HOME__ substitution failed" >&2
        exit 1
    fi
fi

# 5. Check override precedence (OLLAMA_MODEL should be llama3:8b from env file)
echo "    [5/5] Checking override precedence..."
if grep -q "OLLAMA_MODEL" "$PLIST"; then
    # Extract the value after OLLAMA_MODEL key (last occurrence should be from override)
    model_val=$(grep "OLLAMA_MODEL" "$PLIST" | tail -1 | grep -o "<string>.*</string>" | sed 's/<string>//;s/<\/string>//')
    if [[ "$model_val" == "llama3:8b" ]]; then
        echo "         ✓ Override precedence works (OLLAMA_MODEL=llama3:8b)"
    else
        echo "         ✗ Override precedence failed (got $model_val, expected llama3:8b)" >&2
        exit 1
    fi
else
    echo "         ✗ OLLAMA_MODEL missing from plist" >&2
    exit 1
fi

# Bonus: check custom variable was added
echo "    [bonus] Checking custom override vars..."
if grep -q "TEST_CUSTOM_VAR" "$PLIST"; then
    echo "         ✓ Custom env var TEST_CUSTOM_VAR added"
else
    echo "         ✗ Custom env var TEST_CUSTOM_VAR missing" >&2
    exit 1
fi

echo ""
echo "M5 install test passed"
exit 0
