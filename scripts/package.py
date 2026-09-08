#!/usr/bin/env python3
"""Prepare a Windows archive and checksum locally; never publish anything."""
import argparse
import hashlib
from pathlib import Path
import struct
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parent.parent
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["x86_64-pc-windows-gnu", "x86_64-pc-windows-msvc"], required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = ROOT / "target" / args.target / "release" / "onenote.exe"
    data = binary.read_bytes()
    # Refuse an incorrectly selected platform or architecture.
    if data[:2] != b"MZ" or len(data) < 64:
        raise ValueError("Expected a Windows PE executable")
    pe = struct.unpack_from("<I", data, 60)[0]
    if data[pe:pe + 4] != b"PE\0\0" or struct.unpack_from("<H", data, pe + 4)[0] != 0x8664:
        raise ValueError("Expected an x86-64 Windows executable")
    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / f"onenote-v{VERSION}-{args.target}.zip"
    files = {"onenote.exe": data}
    for name in ["README.md", "LICENSE", "CHANGELOG.md"]:
        files[name] = (ROOT / name).read_bytes()
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
        for name, content in files.items():
            bundle.writestr(name, content)
    with zipfile.ZipFile(archive) as bundle:
        if bundle.testzip() is not None or set(bundle.namelist()) != set(files):
            raise ValueError("Archive verification failed")
        for name, content in files.items():
            if bundle.read(name) != content:
                raise ValueError(f"Archive content mismatch: {name}")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".zip.sha256").write_text(f"{digest}  {archive.name}\n")
    print(archive)
    print(f"SHA256 {digest}")


if __name__ == "__main__":
    main()
