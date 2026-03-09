#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# --- Check for web-ext ---
if ! command -v web-ext &>/dev/null; then
    echo "Error: 'web-ext' is not installed."
    echo "Install it with:  npm install -g web-ext"
    exit 1
fi

# --- Resolve AMO credentials ---
AMO_JWT_ISSUER="${AMO_JWT_ISSUER:-}"
AMO_JWT_SECRET="${AMO_JWT_SECRET:-}"

if [[ -z "$AMO_JWT_ISSUER" || -z "$AMO_JWT_SECRET" ]]; then
    CREDS_FILE="$SCRIPT_DIR/.amo-credentials"
    if [[ -f "$CREDS_FILE" ]]; then
        AMO_JWT_ISSUER="$(sed -n '1p' "$CREDS_FILE")"
        AMO_JWT_SECRET="$(sed -n '2p' "$CREDS_FILE")"
        echo "Loaded AMO credentials from .amo-credentials"
    else
        echo "Error: AMO API credentials not found."
        echo ""
        echo "Provide them via environment variables:"
        echo "  export AMO_JWT_ISSUER='your-issuer-key'"
        echo "  export AMO_JWT_SECRET='your-secret-key'"
        echo ""
        echo "Or create a .amo-credentials file in this directory with two lines:"
        echo "  <issuer>"
        echo "  <secret>"
        echo ""
        echo "Get your API keys at: https://addons.mozilla.org/developers/addon/api/key/"
        exit 1
    fi
fi

# --- Add .gitignore entries ---
GITIGNORE="$SCRIPT_DIR/../.gitignore"
if [[ -f "$GITIGNORE" ]]; then
    grep -qxF 'extension/.amo-credentials' "$GITIGNORE" || echo 'extension/.amo-credentials' >> "$GITIGNORE"
    grep -qxF 'extension/web-ext-artifacts/' "$GITIGNORE" || echo 'extension/web-ext-artifacts/' >> "$GITIGNORE"
fi

# --- Build ---
echo "Building extension..."
web-ext build --source-dir="$SCRIPT_DIR" --overwrite-dest
echo ""

# --- Sign ---
echo "Signing extension..."
SIGN_OUTPUT=$(web-ext sign \
    --source-dir="$SCRIPT_DIR" \
    --channel=unlisted \
    --api-key="$AMO_JWT_ISSUER" \
    --api-secret="$AMO_JWT_SECRET" \
    2>&1) || {
    echo "$SIGN_OUTPUT"
    echo ""
    echo "Signing failed. Check your AMO credentials and try again."
    exit 1
}

echo "$SIGN_OUTPUT"
echo ""

# --- Locate the signed .xpi ---
XPI_FILE=$(find "$SCRIPT_DIR/web-ext-artifacts" -name '*.xpi' -type f -printf '%T@ %p\n' 2>/dev/null \
    | sort -rn | head -1 | cut -d' ' -f2-)

if [[ -n "$XPI_FILE" ]]; then
    echo "Signed extension ready:"
    echo "  $XPI_FILE"
else
    echo "Warning: Could not locate .xpi file in web-ext-artifacts/."
    echo "Check the output above for the signed file location."
fi
