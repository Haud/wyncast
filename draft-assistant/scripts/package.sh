#!/usr/bin/env bash
set -euo pipefail

# Build & package orchestrator for draft-assistant
# Produces a distributable archive (.tar.gz or .zip)

# --- Parse arguments ---
TARGET=""
SKIP_EXTENSIONS=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            TARGET="$2"
            shift 2
            ;;
        --skip-extensions)
            SKIP_EXTENSIONS=true
            shift
            ;;
        *)
            echo "Unknown argument: $1"
            echo "Usage: $0 [--target <triple>] [--skip-extensions]"
            exit 1
            ;;
    esac
done

# Default target: host triple
if [[ -z "$TARGET" ]]; then
    TARGET="$(rustc -vV | grep host | awk '{print $2}')"
fi

# --- Project root (parent of scripts/) ---
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

# --- Extract version from Cargo.toml ---
VERSION="$(grep '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"

echo "==> Building draft-assistant v${VERSION} for ${TARGET}"

# --- Determine platform details ---
IS_WINDOWS=false
EXE_SUFFIX=""
if [[ "$TARGET" == *"windows"* ]]; then
    IS_WINDOWS=true
    EXE_SUFFIX=".exe"
fi

# --- Build the binary ---
echo "==> Running cargo build --release --target ${TARGET}"
cargo build --release --target "$TARGET"

# --- Build extensions ---
if [[ "$SKIP_EXTENSIONS" == false ]]; then
    echo "==> Building Chrome extension"
    python3 extension/build.py chrome

    echo "==> Assembling Firefox extension (unsigned)"
    python3 -c "import sys; sys.path.insert(0, 'extension'); from build import assemble_dist; assemble_dist('firefox')"
else
    echo "==> Skipping extension builds (--skip-extensions)"
fi

# --- Stage files ---
DIST_NAME="draft-assistant-${VERSION}-${TARGET}"
DIST_DIR="dist/${DIST_NAME}"

echo "==> Staging files into ${DIST_DIR}/"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR/bin"
mkdir -p "$DIST_DIR/data/projections"

# Binary
cp "target/${TARGET}/release/draft-assistant${EXE_SUFFIX}" "$DIST_DIR/bin/draft-assistant${EXE_SUFFIX}"

# Projection data
cp data/projections/hitters.csv "$DIST_DIR/data/projections/hitters.csv"
cp data/projections/pitchers.csv "$DIST_DIR/data/projections/pitchers.csv"

# Extensions
if [[ "$SKIP_EXTENSIONS" == false ]]; then
    mkdir -p "$DIST_DIR/extensions"
    cp -r extension/dist/firefox "$DIST_DIR/extensions/firefox"
    cp -r extension/dist/chrome "$DIST_DIR/extensions/chrome"
fi

# Installer script
if [[ "$IS_WINDOWS" == true ]]; then
    cp scripts/install.ps1 "$DIST_DIR/install.ps1"
else
    cp scripts/install.sh "$DIST_DIR/install.sh"
    chmod +x "$DIST_DIR/install.sh"
fi

# --- Create archive ---
echo "==> Creating archive"
cd dist

if [[ "$IS_WINDOWS" == true ]]; then
    ARCHIVE="${DIST_NAME}.zip"
    zip -r "$ARCHIVE" "$DIST_NAME"
else
    ARCHIVE="${DIST_NAME}.tar.gz"
    tar czf "$ARCHIVE" "$DIST_NAME"
fi

cd "$PROJECT_DIR"

ARCHIVE_PATH="dist/${ARCHIVE}"
ARCHIVE_SIZE="$(du -h "$ARCHIVE_PATH" | awk '{print $1}')"

echo ""
echo "==> Archive ready:"
echo "    Path: ${ARCHIVE_PATH}"
echo "    Size: ${ARCHIVE_SIZE}"
