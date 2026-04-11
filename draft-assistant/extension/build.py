#!/usr/bin/env python3
"""Build the Wyndham Draft Sync browser extension for Firefox and/or Chrome.

Usage:
    python build.py firefox     # Build Firefox extension (+ sign via AMO)
    python build.py chrome      # Build Chrome extension (unpacked / zip)
    python build.py all         # Build both

Cross-platform (Windows, macOS, Linux).

Requirements:
    - Python 3.6+
    - web-ext (npm install -g web-ext) — Firefox only
    - AMO API credentials (env vars, .env file, or .amo-credentials file) — Firefox only
"""

import shutil
import subprocess
import sys
from pathlib import Path

EXTENSION_DIR = Path(__file__).resolve().parent
PROJECT_DIR = EXTENSION_DIR.parent
DIST_DIR = EXTENSION_DIR / "dist"
CREDENTIALS_FILE = EXTENSION_DIR / ".amo-credentials"
ENV_FILE = PROJECT_DIR / ".env"

# Shared source files (at extension root) copied into Chrome dist builds.
# Firefox loads directly from the extension root, so no copy needed.
SHARED_FILES = [
    ("browser-polyfill.js", "browser-polyfill.js"),
    ("background-core.js", "background-core.js"),
    ("content_scripts/espn.js", "content_scripts/espn.js"),
    ("content_scripts/espn-matchup.js", "content_scripts/espn-matchup.js"),
]

# Firefox files live at extension root (no separate directory needed).
FIREFOX_FILES = [
    "manifest.json",
    "background.js",
]

# Only Chrome needs browser-specific file listings for dist assembly.
CHROME_FILES = [
    "manifest.json",
    "background.js",
    "offscreen.html",
    "offscreen.js",
]


def check_web_ext() -> str:
    """Check that web-ext is available on PATH. Return path to executable."""
    path = shutil.which("web-ext")
    if path is None:
        print("Error: 'web-ext' not found on PATH.", file=sys.stderr)
        print("Install it with: npm install -g web-ext", file=sys.stderr)
        sys.exit(1)
    return path


def parse_env_file(path: Path) -> dict:
    """Parse a .env file into a dict of key-value pairs.

    Supports KEY=VALUE lines. Ignores comments (#) and blank lines.
    Strips optional surrounding quotes from values.
    """
    env = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip()
        # Strip matching surrounding quotes
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
            value = value[1:-1]
        env[key] = value
    return env


def load_credentials() -> tuple:
    """Load AMO API credentials from env vars, .env file, or .amo-credentials file.

    Lookup order:
      1. Environment variables (AMO_JWT_ISSUER, AMO_JWT_SECRET)
      2. .env file in draft-assistant/ (same keys)
      3. Legacy .amo-credentials file in extension/

    Returns (issuer, secret) tuple.
    """
    import os

    issuer = os.environ.get("AMO_JWT_ISSUER")
    secret = os.environ.get("AMO_JWT_SECRET")

    if issuer and secret:
        return issuer, secret

    if ENV_FILE.exists():
        env = parse_env_file(ENV_FILE)
        issuer = env.get("AMO_JWT_ISSUER")
        secret = env.get("AMO_JWT_SECRET")
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
    print("  2. .env file (draft-assistant/.env):", file=sys.stderr)
    print("     AMO_JWT_ISSUER=your-api-key", file=sys.stderr)
    print("     AMO_JWT_SECRET=your-api-secret", file=sys.stderr)
    print("", file=sys.stderr)
    print("  3. Credentials file (extension/.amo-credentials):", file=sys.stderr)
    print("     Line 1: API key (JWT issuer)", file=sys.stderr)
    print("     Line 2: API secret", file=sys.stderr)
    print("", file=sys.stderr)
    print(
        "  Get your API keys at: https://addons.mozilla.org/developers/addon/api/key/",
        file=sys.stderr,
    )
    sys.exit(1)


def run_command(args: list, description: str, cwd: str = None) -> None:
    """Run a subprocess command. Print output and exit on failure."""
    print(f"\n{'='*60}")
    print(f"  {description}")
    print(f"{'='*60}\n")

    result = subprocess.run(
        args,
        cwd=cwd or str(EXTENSION_DIR),
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


def assemble_firefox_dist() -> Path:
    """Assemble the dist directory for Firefox.

    Copies shared files (from extension root) and Firefox-specific files
    into dist/firefox/. Returns the path to the dist directory.
    """
    dist = DIST_DIR / "firefox"
    if dist.exists():
        shutil.rmtree(dist)
    dist.mkdir(parents=True)

    # Copy shared files from extension root
    for src_rel, dst_rel in SHARED_FILES:
        src = EXTENSION_DIR / src_rel
        dst = dist / dst_rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)

    # Copy icons directory (if it has real files beyond .gitkeep)
    icons_src = EXTENSION_DIR / "icons"
    if icons_src.exists():
        icons_dst = dist / "icons"
        icons_dst.mkdir(parents=True, exist_ok=True)
        for icon_file in icons_src.iterdir():
            if icon_file.name != ".gitkeep":
                shutil.copy2(icon_file, icons_dst / icon_file.name)

    # Copy Firefox-specific files (from extension root)
    for filename in FIREFOX_FILES:
        src = EXTENSION_DIR / filename
        dst = dist / filename
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)

    print(f"Assembled Firefox extension in {dist}")
    return dist


def assemble_chrome_dist() -> Path:
    """Assemble the dist directory for Chrome.

    Copies shared files (from extension root) and Chrome-specific files
    into dist/chrome/. Returns the path to the dist directory.
    """
    dist = DIST_DIR / "chrome"
    if dist.exists():
        shutil.rmtree(dist)
    dist.mkdir(parents=True)

    # Copy shared files from extension root
    for src_rel, dst_rel in SHARED_FILES:
        src = EXTENSION_DIR / src_rel
        dst = dist / dst_rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)

    # Copy icons directory (if it has real files beyond .gitkeep)
    icons_src = EXTENSION_DIR / "icons"
    if icons_src.exists():
        icons_dst = dist / "icons"
        icons_dst.mkdir(parents=True, exist_ok=True)
        for icon_file in icons_src.iterdir():
            if icon_file.name != ".gitkeep":
                shutil.copy2(icon_file, icons_dst / icon_file.name)

    # Copy Chrome-specific files
    chrome_dir = EXTENSION_DIR / "chrome"
    for filename in CHROME_FILES:
        src = chrome_dir / filename
        dst = dist / filename
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)

    print(f"Assembled Chrome extension in {dist}")
    return dist


def build_firefox() -> None:
    """Build and sign the Firefox extension.

    Firefox files live at the extension root, so web-ext runs directly
    against EXTENSION_DIR with --ignore-files to exclude non-extension files.
    """
    web_ext = check_web_ext()
    issuer, secret = load_credentials()

    artifacts_dir = DIST_DIR / "firefox" / "web-ext-artifacts"
    artifacts_dir.mkdir(parents=True, exist_ok=True)

    ignore_files = [
        "chrome/",
        "dist/",
        "*.py",
        "*.pyc",
        "__pycache__",
        "icons/.gitkeep",
        "BUILD.md",
        ".amo-credentials",
        ".gitkeep",
    ]

    # Build
    build_args = [
        web_ext, "build",
        f"--source-dir={EXTENSION_DIR}",
        "--overwrite-dest",
        f"--artifacts-dir={artifacts_dir}",
    ]
    for pattern in ignore_files:
        build_args.append(f"--ignore-files={pattern}")

    run_command(build_args, "Building Firefox extension")

    # Sign
    sign_args = [
        web_ext,
        "sign",
        f"--source-dir={EXTENSION_DIR}",
        "--channel=unlisted",
        f"--api-key={issuer}",
        f"--api-secret={secret}",
        f"--artifacts-dir={artifacts_dir}",
    ]
    for pattern in ignore_files:
        sign_args.append(f"--ignore-files={pattern}")

    run_command(sign_args, "Signing Firefox extension")

    # Report result
    xpi_files = list(artifacts_dir.glob("*.xpi")) if artifacts_dir.exists() else []
    if xpi_files:
        print(f"\nSigned Firefox extension: {xpi_files[0]}")
    else:
        print("\nWarning: No .xpi file found after signing.", file=sys.stderr)


def build_chrome() -> None:
    """Build the Chrome extension (produces unpacked directory for developer mode)."""
    dist = assemble_chrome_dist()
    print(f"\nChrome extension ready at: {dist}")
    print("Load it in Chrome via chrome://extensions (developer mode, 'Load unpacked').")


def main() -> None:
    if len(sys.argv) < 2:
        print("Usage: python build.py <firefox|chrome|all>", file=sys.stderr)
        sys.exit(1)

    target = sys.argv[1].lower()

    if target == "firefox":
        build_firefox()
    elif target == "chrome":
        build_chrome()
    elif target == "all":
        build_firefox()
        build_chrome()
    else:
        print(f"Unknown target: {target}", file=sys.stderr)
        print("Usage: python build.py <firefox|chrome|all>", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
