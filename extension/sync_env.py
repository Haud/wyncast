#!/usr/bin/env python3
"""Sync AMO credentials from 1Password to draft-assistant/.env."""

import shutil
import subprocess
import sys
from pathlib import Path

PROJECT_DIR = Path(__file__).resolve().parent.parent
ENV_FILE = PROJECT_DIR / ".env"

OP_VAULT = "Private"
OP_ITEM = "Wyncast Firefox"
OP_FIELDS = {
    "AMO_JWT_ISSUER": "username",
    "AMO_JWT_SECRET": "credential",
}


def check_op() -> str:
    """Check that the 1Password CLI is available on PATH."""
    path = shutil.which("op")
    if path is None:
        print("Error: '1Password CLI (op)' not found on PATH.", file=sys.stderr)
        print("Install it with: brew install 1password-cli", file=sys.stderr)
        sys.exit(1)
    return path


def read_field(op: str, field: str) -> str:
    """Read a single field from the 1Password item."""
    ref = f"op://{OP_VAULT}/{OP_ITEM}/{field}"
    result = subprocess.run(
        [op, "read", ref],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f"Error reading '{ref}': {result.stderr.strip()}", file=sys.stderr)
        sys.exit(1)
    return result.stdout.strip()


def main() -> None:
    op = check_op()

    lines = []
    for env_key, op_field in OP_FIELDS.items():
        value = read_field(op, op_field)
        lines.append(f"{env_key}={value}")

    ENV_FILE.write_text("\n".join(lines) + "\n")
    print(f"Credentials written to {ENV_FILE}")


if __name__ == "__main__":
    main()
