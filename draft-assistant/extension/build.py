#!/usr/bin/env python3
"""Build and sign the Wyndham Draft Sync Firefox extension.

Cross-platform (Windows, macOS, Linux) script that uses web-ext to build
and sign the extension for unlisted distribution via AMO.

Requirements:
    - Python 3.6+
    - web-ext (npm install -g web-ext)
    - AMO API credentials (env vars or .amo-credentials file)
"""

import shutil
import subprocess
import sys
from pathlib import Path

EXTENSION_DIR = Path(__file__).resolve().parent
CREDENTIALS_FILE = EXTENSION_DIR / ".amo-credentials"
ARTIFACTS_DIR = EXTENSION_DIR / "web-ext-artifacts"


def check_web_ext() -> str:
    """Check that web-ext is available on PATH. Return path to executable."""
    path = shutil.which("web-ext")
    if path is None:
        print("Error: 'web-ext' not found on PATH.", file=sys.stderr)
        print("Install it with: npm install -g web-ext", file=sys.stderr)
        sys.exit(1)
    return path


def load_credentials() -> tuple:
    """Load AMO API credentials from env vars or .amo-credentials file.

    Returns (issuer, secret) tuple.
    """
    import os

    issuer = os.environ.get("AMO_JWT_ISSUER")
    secret = os.environ.get("AMO_JWT_SECRET")

    if issuer and secret:
        return issuer, secret

    if CREDENTIALS_FILE.exists():
        lines = CREDENTIALS_FILE.read_text().strip().splitlines()
        if len(lines) >= 2:
            issuer = lines[0].strip()
            secret = lines[1].strip()
            if issuer and secret:
                return issuer, secret

    print("Error: AMO API credentials not found.", file=sys.stderr)
    print("", file=sys.stderr)
    print("Provide credentials using one of these methods:", file=sys.stderr)
    print("", file=sys.stderr)
    print("  1. Environment variables:", file=sys.stderr)
    print("     export AMO_JWT_ISSUER='your-api-key'", file=sys.stderr)
    print("     export AMO_JWT_SECRET='your-api-secret'", file=sys.stderr)
    print("", file=sys.stderr)
    print("  2. Credentials file (extension/.amo-credentials):", file=sys.stderr)
    print("     Line 1: API key (JWT issuer)", file=sys.stderr)
    print("     Line 2: API secret", file=sys.stderr)
    print("", file=sys.stderr)
    print(
        "  Get your API keys at: https://addons.mozilla.org/developers/addon/api/key/",
        file=sys.stderr,
    )
    sys.exit(1)


def run_command(args: list, description: str) -> None:
    """Run a subprocess command. Print output and exit on failure."""
    print(f"\n{'='*60}")
    print(f"  {description}")
    print(f"{'='*60}\n")

    result = subprocess.run(
        args,
        cwd=str(EXTENSION_DIR),
        capture_output=True,
        text=True,
    )

    if result.stdout:
        print(result.stdout)
    if result.stderr:
        print(result.stderr, file=sys.stderr)

    if result.returncode != 0:
        print(
            f"\nError: {description} failed (exit code {result.returncode}).",
            file=sys.stderr,
        )
        sys.exit(result.returncode)


def find_xpi() -> Path:
    """Find the signed .xpi file in web-ext-artifacts/."""
    if not ARTIFACTS_DIR.exists():
        print("Error: web-ext-artifacts/ directory not found.", file=sys.stderr)
        sys.exit(1)

    xpi_files = sorted(ARTIFACTS_DIR.glob("*.xpi"), key=lambda p: p.stat().st_mtime)
    if not xpi_files:
        print("Error: No .xpi file found in web-ext-artifacts/.", file=sys.stderr)
        sys.exit(1)

    return xpi_files[-1]


def main() -> None:
    web_ext = check_web_ext()
    issuer, secret = load_credentials()

    source_dir = str(EXTENSION_DIR)

    # Build
    run_command(
        [web_ext, "build", f"--source-dir={source_dir}", "--overwrite-dest"],
        "Building extension",
    )

    # Sign
    run_command(
        [
            web_ext,
            "sign",
            f"--source-dir={source_dir}",
            "--channel=unlisted",
            f"--api-key={issuer}",
            f"--api-secret={secret}",
        ],
        "Signing extension",
    )

    # Report result
    xpi = find_xpi()
    print(f"\nSigned extension: {xpi}")


if __name__ == "__main__":
    main()
