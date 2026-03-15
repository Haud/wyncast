#!/usr/bin/env bash
set -euo pipefail

# wyncast installer for macOS / Linux

# --- Colors ---
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# --- Detect OS ---
OS="$(uname -s)"
case "$OS" in
    Darwin) PLATFORM="macos" ;;
    Linux)  PLATFORM="linux" ;;
    *)
        echo -e "${RED}Unsupported OS: ${OS}${NC}"
        exit 1
        ;;
esac

# --- Paths ---
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="$HOME/.local/bin"

if [[ "$PLATFORM" == "macos" ]]; then
    DATA_DIR="$HOME/Library/Application Support/wyncast"
else
    DATA_DIR="$HOME/.local/share/wyncast"
fi

echo -e "${BLUE}Installing wyncast (${PLATFORM})...${NC}"
echo ""

# --- Install binary ---
mkdir -p "$BIN_DIR"
cp "$SCRIPT_DIR/bin/wyncast" "$BIN_DIR/wyncast"
chmod +x "$BIN_DIR/wyncast"
echo -e "${GREEN}✓${NC} Installed binary to ${BIN_DIR}/wyncast"

# --- macOS: clear quarantine ---
if [[ "$PLATFORM" == "macos" ]]; then
    if command -v xattr &>/dev/null; then
        xattr -dr com.apple.quarantine "$BIN_DIR/wyncast" 2>/dev/null || true
        echo -e "${GREEN}✓${NC} Cleared macOS quarantine flag"
    fi
fi

# --- Add to PATH ---
add_to_path() {
    local rcfile="$1"
    if [[ -f "$rcfile" ]] && grep -q '/.local/bin' "$rcfile"; then
        echo -e "${GREEN}✓${NC} PATH already configured in $(basename "$rcfile")"
    else
        echo '' >> "$rcfile"
        echo '# Added by wyncast installer' >> "$rcfile"
        echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$rcfile"
        echo -e "${GREEN}✓${NC} Added ~/.local/bin to PATH in $(basename "$rcfile")"
        echo -e "${YELLOW}  Restart your shell or run: source ${rcfile}${NC}"
    fi
}

if [[ "$PLATFORM" == "macos" ]]; then
    add_to_path "$HOME/.zshrc"
else
    if [[ -f "$HOME/.zshrc" ]]; then
        add_to_path "$HOME/.zshrc"
    elif [[ -f "$HOME/.bashrc" ]]; then
        add_to_path "$HOME/.bashrc"
    else
        add_to_path "$HOME/.bashrc"
    fi
fi

# --- Copy projection data ---
mkdir -p "$DATA_DIR/data/projections"
cp "$SCRIPT_DIR/data/projections/hitters.csv" "$DATA_DIR/data/projections/hitters.csv"
cp "$SCRIPT_DIR/data/projections/pitchers.csv" "$DATA_DIR/data/projections/pitchers.csv"
echo -e "${GREEN}✓${NC} Installed projection data to ${DATA_DIR}/data/projections/"

# --- Copy extensions ---
if [[ -d "$SCRIPT_DIR/extensions" ]]; then
    mkdir -p "$DATA_DIR/extensions"
    if [[ -d "$SCRIPT_DIR/extensions/firefox" ]]; then
        rm -rf "$DATA_DIR/extensions/firefox"
        cp -r "$SCRIPT_DIR/extensions/firefox" "$DATA_DIR/extensions/firefox"
        echo -e "${GREEN}✓${NC} Installed Firefox extension to ${DATA_DIR}/extensions/firefox/"
    fi
    if [[ -d "$SCRIPT_DIR/extensions/chrome" ]]; then
        rm -rf "$DATA_DIR/extensions/chrome"
        cp -r "$SCRIPT_DIR/extensions/chrome" "$DATA_DIR/extensions/chrome"
        echo -e "${GREEN}✓${NC} Installed Chrome extension to ${DATA_DIR}/extensions/chrome/"
    fi
else
    echo -e "${YELLOW}⚠ No extensions found in archive (built with --skip-extensions?)${NC}"
fi

# --- Post-install instructions ---
echo ""
echo -e "${BLUE}────────────────────────────────────────${NC}"
echo -e "${GREEN}Installation complete!${NC}"
echo -e "${BLUE}────────────────────────────────────────${NC}"
echo ""
echo -e "${BLUE}Browser Extension Setup:${NC}"
echo ""
echo -e "  ${YELLOW}Firefox:${NC}"
echo "    1. Open about:debugging in Firefox"
echo "    2. Click 'This Firefox' → 'Load Temporary Add-on'"
echo "    3. Select manifest.json from:"
echo "       ${DATA_DIR}/extensions/firefox/"
echo ""
echo -e "  ${YELLOW}Chrome:${NC}"
echo "    1. Open chrome://extensions"
echo "    2. Enable 'Developer mode' (top right)"
echo "    3. Click 'Load unpacked' and select:"
echo "       ${DATA_DIR}/extensions/chrome/"
echo ""
echo -e "${BLUE}Notes:${NC}"
echo "  - Config file auto-generates on first run"
echo "  - Run 'wyncast' to start"
