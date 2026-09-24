#!/usr/bin/env python3
"""Stage only expected development packages, with provenance and checksums.

This records build outputs; it does not certify signing, feature parity, or audio.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil


PACKAGES = {
    "linux-x64": ("Caper-linux-x64.deb", "Caper-linux-x64.tar.gz"),
    "windows-x64": ("Caper-windows-x64.zip",),
    "macos-arm64": ("Caper-macos-arm64.zip",),
    "macos-x64": ("Caper-macos-x64.zip",),
    "ios-simulator-arm64": ("Caper-ios-simulator-arm64.zip",),
    "android-debug": ("Caper-android-debug.apk",),
}


def stage(target: str, revision: str, source: Path, destination: Path) -> None:
    if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", revision):
        raise ValueError("revision must be the full lowercase Git object ID")
    packages = []
    for name in PACKAGES[target]:
        path = source / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"Missing, empty, or symlinked package: {name}")
        with path.open("rb") as package:
            digest = hashlib.file_digest(package, "sha256").hexdigest()
        packages.append({"name": name, "bytes": path.stat().st_size, "sha256": digest})

    # Never silently include packages from an earlier build or arbitrary files.
    destination.mkdir(parents=True, exist_ok=False)
    for package in packages:
        shutil.copyfile(source / package["name"], destination / package["name"])
    manifest = {
        "schemaVersion": 1,
        "revision": revision,
        "target": target,
        "channel": "development",
        "distribution": (
            "simulator-only" if target.startswith("ios-simulator")
            else "debug-signed" if target == "android-debug"
            else "not-distribution-signed"
        ),
        "featureParityVerified": False,
        "nativeVoiceVerified": False,
        "packages": packages,
    }
    (destination / "BUILD.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    (destination / "SHA256SUMS").write_text(
        "".join(f"{p['sha256']}  {p['name']}\n" for p in packages), encoding="utf-8"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=PACKAGES)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    args = parser.parse_args()
    stage(args.target, args.revision, args.source, args.destination)
