#!/usr/bin/env python3
"""Validate registry packages and test wheel installation without source builds."""
import argparse
import email
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parent.parent
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
PACKAGE = "onenote-cli-rs"


def wheels(directory):
    return sorted(directory.glob(f"onenote_cli_rs-{VERSION}-*.whl"))


def inspect_wheel(path):
    with zipfile.ZipFile(path) as archive:
        metadata = [n for n in archive.namelist() if n.endswith(".dist-info/METADATA")]
        binaries = [n for n in archive.namelist() if n.endswith((".data/scripts/onenote", ".data/scripts/onenote.exe"))]
        if len(metadata) != 1 or len(binaries) != 1 or archive.testzip() is not None:
            raise ValueError(f"Invalid wheel contents: {path.name}")
        fields = email.message_from_bytes(archive.read(metadata[0]))
        if fields["Name"].replace("_", "-") != PACKAGE or fields["Version"] != VERSION:
            raise ValueError(f"Wheel metadata mismatch: {path.name}")
        platform = path.stem.split("-")[-1]
        if platform == "win_amd64" and binaries[0].endswith(".exe"):
            return "windows"
        if "manylinux2014_x86_64" in platform.split(".") and not binaries[0].endswith(".exe"):
            return "linux"
        raise ValueError(f"Unexpected wheel platform: {path.name}")


def smoke(directory):
    candidates = wheels(directory)
    if len(candidates) != 1:
        raise ValueError("Smoke test requires exactly one wheel")
    inspect_wheel(candidates[0])
    with tempfile.TemporaryDirectory(prefix="onenote-wheel-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, UV_TOOL_DIR=str(root / "tools"),
                   UV_TOOL_BIN_DIR=str(root / "bin"), UV_CACHE_DIR=str(root / "cache"),
                   UV_PYTHON_DOWNLOADS="never")
        subprocess.run(["uv", "tool", "install", "--no-build", "--no-index", "--python", sys.executable,
                        "--find-links", str(directory.resolve()), f"{PACKAGE}=={VERSION}"], env=env, check=True)
        binary = root / "bin" / ("onenote.exe" if os.name == "nt" else "onenote")
        if subprocess.check_output([binary, "--version"], text=True).strip() != f"onenote {VERSION}":
            raise ValueError("Installed version mismatch")
        spec = json.loads(subprocess.check_output([binary, "schema"], text=True))
        if spec["version"] != VERSION or "--scan" not in json.dumps(spec):
            raise ValueError("Installed CLI contract mismatch")
        # Also check the raw executable outside its wheel installation tree.
        with zipfile.ZipFile(candidates[0]) as archive:
            name = next(n for n in archive.namelist() if n.endswith((".data/scripts/onenote", ".data/scripts/onenote.exe")))
            standalone = root / Path(name).name
            standalone.write_bytes(archive.read(name))
            standalone.chmod(0o755)
        subprocess.run([standalone, "doctor", "--offline"], check=True, stdout=subprocess.DEVNULL)
    print(f"Installed and executed {candidates[0].name} without Rust or source builds")


def verify(directory, tag):
    if tag is not None and tag != f"v{VERSION}":
        raise ValueError(f"Tag must be v{VERSION}")
    candidates = wheels(directory)
    if sorted(inspect_wheel(p) for p in candidates) != ["linux", "windows"]:
        raise ValueError("Release requires exactly one Windows x64 and one Linux x64 wheel")
    source = directory / f"onenote_cli_rs-{VERSION}.tar.gz"
    crate = directory / f"onenote-cli-{VERSION}.crate"
    for archive_path, prefix in [(source, f"onenote_cli_rs-{VERSION}"), (crate, f"onenote-cli-{VERSION}")]:
        with tarfile.open(archive_path) as archive:
            manifest = archive.extractfile(f"{prefix}/Cargo.toml")
            if manifest is None or tomllib.loads(manifest.read().decode())["package"]["version"] != VERSION:
                raise ValueError(f"Source version mismatch: {archive_path.name}")
    expected = set(candidates) | {source, crate}
    if set(directory.iterdir()) - expected - {directory / "SHA256SUMS"}:
        raise ValueError("Unexpected release files")
    checksums = "".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in sorted(expected))
    checksum_path = directory / "SHA256SUMS"
    if checksum_path.exists() and checksum_path.read_text() != checksums:
        raise ValueError("Package checksums changed after verification")
    checksum_path.write_text(checksums)
    print("Verified Windows and Linux wheels, Python source distribution, and Cargo package")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("smoke").add_argument("directory", type=Path)
    verify_parser = sub.add_parser("verify")
    verify_parser.add_argument("directory", type=Path)
    verify_parser.add_argument("--require-tag")
    args = parser.parse_args()
    if args.command == "smoke":
        smoke(args.directory)
    else:
        verify(args.directory, args.require_tag)


if __name__ == "__main__":
    main()
