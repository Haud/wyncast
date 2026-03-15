#!/usr/bin/env python3
"""Build & package orchestrator for draft-assistant.

Produces a distributable archive (.tar.gz or .zip) containing the binary,
projection data, browser extensions, and installer script.

Cross-platform: works on Windows, macOS, and Linux.
Requires: Python 3.6+, stdlib only (no third-party packages).

Usage:
    python scripts/package.py [--target <triple>] [--skip-extensions]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path


def get_host_triple():
    """Get the host target triple from rustc."""
    result = subprocess.run(
        ["rustc", "-vV"],
        capture_output=True,
        text=True,
        check=True,
    )
    for line in result.stdout.splitlines():
        if line.startswith("host:"):
            return line.split(":", 1)[1].strip()
    print("Error: could not determine host triple from 'rustc -vV'", file=sys.stderr)
    sys.exit(1)


def extract_version(cargo_toml):
    """Extract the version string from Cargo.toml's [package] section."""
    text = cargo_toml.read_text()
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        print("Error: could not extract version from Cargo.toml", file=sys.stderr)
        sys.exit(1)
    return match.group(1)


def format_size(size_bytes):
    """Format a file size in human-readable form (KB or MB)."""
    if size_bytes >= 1024 * 1024:
        return "{:.1f} MB".format(size_bytes / (1024 * 1024))
    else:
        return "{:.1f} KB".format(size_bytes / 1024)


def build_binary(project_dir, target):
    """Run cargo build --release for the given target."""
    print("==> Running cargo build --release --target {}".format(target))
    result = subprocess.run(
        ["cargo", "build", "--release", "--target", target],
        cwd=str(project_dir),
    )
    if result.returncode != 0:
        print("Error: cargo build failed (exit code {})".format(result.returncode),
              file=sys.stderr)
        sys.exit(result.returncode)


def build_extensions(project_dir):
    """Build Chrome and Firefox extensions."""
    python = sys.executable

    # Chrome: run extension/build.py chrome
    print("==> Building Chrome extension")
    result = subprocess.run(
        [python, str(project_dir / "extension" / "build.py"), "chrome"],
        cwd=str(project_dir),
    )
    if result.returncode != 0:
        print("Error: Chrome extension build failed (exit code {})".format(
            result.returncode), file=sys.stderr)
        sys.exit(result.returncode)

    # Firefox: assemble without signing (import directly)
    print("==> Assembling Firefox extension (unsigned)")
    ext_dir = str(project_dir / "extension")
    if ext_dir not in sys.path:
        sys.path.insert(0, ext_dir)
    from build import assemble_dist
    assemble_dist("firefox")


def stage_files(project_dir, dist_dir, target, is_windows, skip_extensions):
    """Copy all distributable files into the staging directory."""
    print("==> Staging files into {}/".format(dist_dir.relative_to(project_dir)))

    # Clean and create staging directory
    if dist_dir.exists():
        shutil.rmtree(dist_dir)

    # Binary
    exe_suffix = ".exe" if is_windows else ""
    bin_dir = dist_dir / "bin"
    bin_dir.mkdir(parents=True)
    src_bin = project_dir / "target" / target / "release" / ("wyncast" + exe_suffix)
    shutil.copy2(str(src_bin), str(bin_dir / ("wyncast" + exe_suffix)))

    # Projection data
    proj_dir = dist_dir / "data" / "projections"
    proj_dir.mkdir(parents=True)
    src_proj = project_dir / "data" / "projections"
    shutil.copy2(str(src_proj / "hitters.csv"), str(proj_dir / "hitters.csv"))
    shutil.copy2(str(src_proj / "pitchers.csv"), str(proj_dir / "pitchers.csv"))

    # Extensions
    if not skip_extensions:
        ext_dst = dist_dir / "extensions"
        ext_dst.mkdir(parents=True)
        ext_src = project_dir / "extension" / "dist"
        shutil.copytree(str(ext_src / "firefox"), str(ext_dst / "firefox"))
        shutil.copytree(str(ext_src / "chrome"), str(ext_dst / "chrome"))

    # Installer script
    scripts_dir = project_dir / "scripts"
    if is_windows:
        shutil.copy2(str(scripts_dir / "install.ps1"), str(dist_dir / "install.ps1"))
    else:
        install_dst = dist_dir / "install.sh"
        shutil.copy2(str(scripts_dir / "install.sh"), str(install_dst))
        os.chmod(str(install_dst), 0o755)


def create_archive(dist_parent, dist_name, is_windows):
    """Create a .tar.gz or .zip archive and return the archive path."""
    print("==> Creating archive")

    if is_windows:
        archive_name = dist_name + ".zip"
        archive_path = dist_parent / archive_name
        with zipfile.ZipFile(str(archive_path), "w", zipfile.ZIP_DEFLATED) as zf:
            dist_dir = dist_parent / dist_name
            for root, dirs, files in os.walk(str(dist_dir)):
                for f in files:
                    file_path = Path(root) / f
                    arcname = str(file_path.relative_to(dist_parent))
                    zf.write(str(file_path), arcname)
    else:
        archive_name = dist_name + ".tar.gz"
        archive_path = dist_parent / archive_name
        with tarfile.open(str(archive_path), "w:gz") as tf:
            tf.add(str(dist_parent / dist_name), arcname=dist_name)

    return archive_path


def main():
    parser = argparse.ArgumentParser(
        description="Build & package draft-assistant for distribution"
    )
    parser.add_argument(
        "--target",
        default=None,
        help="Rust target triple (default: host triple from rustc -vV)",
    )
    parser.add_argument(
        "--skip-extensions",
        action="store_true",
        help="Skip building browser extensions",
    )
    args = parser.parse_args()

    # Resolve target
    target = args.target if args.target else get_host_triple()

    # Project root is parent of scripts/
    project_dir = Path(__file__).resolve().parent.parent

    # Extract version
    version = extract_version(project_dir / "Cargo.toml")

    print("==> Building draft-assistant v{} for {}".format(version, target))

    is_windows = "windows" in target

    # Build binary
    build_binary(project_dir, target)

    # Build extensions
    if not args.skip_extensions:
        build_extensions(project_dir)
    else:
        print("==> Skipping extension builds (--skip-extensions)")

    # Stage files
    dist_name = "draft-assistant-{}-{}".format(version, target)
    dist_dir = project_dir / "dist" / dist_name
    stage_files(project_dir, dist_dir, target, is_windows, args.skip_extensions)

    # Create archive
    archive_path = create_archive(project_dir / "dist", dist_name, is_windows)

    # Report
    archive_size = format_size(os.stat(str(archive_path)).st_size)
    rel_archive = archive_path.relative_to(project_dir)
    print("")
    print("==> Archive ready:")
    print("    Path: {}".format(rel_archive))
    print("    Size: {}".format(archive_size))


if __name__ == "__main__":
    main()
