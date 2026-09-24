#!/usr/bin/env python3
"""Fetch and verify Caper's pinned LiveKit libwebrtc prebuilt archive."""

from __future__ import annotations

import argparse
import hashlib
import os
import platform
import shutil
import tempfile
import urllib.request
import zipfile
from pathlib import Path

TAG = "webrtc-89d790b"
ARCHIVES = {
    "linux": (
        "webrtc-linux-x64-release.zip",
        "b167adad5291cea0e4d66a0454d9d52d2ad714e6b0ed70f4410317d3ebde70c5",
    ),
    "windows": (
        "webrtc-win-x64-release.zip",
        "5c2349c960bae4f06f71c102f58552d0d09c912a142811acfcadfb7c883cbf58",
    ),
}
MAX_ARCHIVE_BYTES = 2_000_000_000
MAX_EXPANDED_BYTES = 4_000_000_000


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def download(url: str, destination: Path) -> None:
    request = urllib.request.Request(url, headers={"User-Agent": "Caper native build"})
    with urllib.request.urlopen(request, timeout=60) as response, destination.open("wb") as output:
        total = 0
        while block := response.read(1024 * 1024):
            total += len(block)
            if total > MAX_ARCHIVE_BYTES:
                raise RuntimeError("libwebrtc archive exceeds the size limit")
            output.write(block)


def extract(archive: Path, destination: Path) -> None:
    temporary = Path(tempfile.mkdtemp(prefix="caper-webrtc-", dir=destination.parent))
    try:
        with zipfile.ZipFile(archive) as bundle:
            total = sum(member.file_size for member in bundle.infolist())
            if total > MAX_EXPANDED_BYTES:
                raise RuntimeError("expanded libwebrtc archive exceeds the size limit")
            root = temporary.resolve()
            for member in bundle.infolist():
                target = (temporary / member.filename).resolve()
                if root != target and root not in target.parents:
                    raise RuntimeError(f"unsafe libwebrtc archive path: {member.filename}")
            bundle.extractall(temporary)
        roots = list(temporary.iterdir())
        extracted = roots[0] if len(roots) == 1 and roots[0].is_dir() else temporary
        if not (extracted / "lib" / "libwebrtc.a").exists() and not (
            extracted / "lib" / "webrtc.lib"
        ).exists():
            raise RuntimeError("libwebrtc archive does not contain the expected library")
        if not (extracted / "LICENSE.md").is_file():
            raise RuntimeError("libwebrtc archive does not contain LICENSE.md")
        if destination.exists():
            shutil.rmtree(destination)
        extracted.replace(destination)
        if temporary.exists():
            temporary.rmdir()
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform", choices=ARCHIVES, default=platform.system().lower())
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if platform.machine().lower() not in {"x86_64", "amd64"}:
        raise SystemExit("Caper voice archives support x64 hosts only")
    filename, expected = ARCHIVES[args.platform]
    output = args.output or Path(__file__).resolve().parents[1] / "target" / "libwebrtc" / filename.removeprefix("webrtc-").removesuffix(".zip")
    output = output.resolve()
    if (output / ".caper-sha256").is_file() and (output / ".caper-sha256").read_text().strip() == expected:
        print(output)
        return
    output.parent.mkdir(parents=True, exist_ok=True)
    archive = output.parent / filename
    if not archive.is_file() or sha256(archive) != expected:
        partial = archive.with_suffix(".partial")
        partial.unlink(missing_ok=True)
        download(f"https://github.com/livekit/rust-sdks/releases/download/{TAG}/{filename}", partial)
        actual = sha256(partial)
        if actual != expected:
            partial.unlink(missing_ok=True)
            raise RuntimeError(f"libwebrtc SHA256 mismatch: expected {expected}, got {actual}")
        os.replace(partial, archive)
    extract(archive, output)
    (output / ".caper-sha256").write_text(expected + "\n")
    print(output)


if __name__ == "__main__":
    main()
