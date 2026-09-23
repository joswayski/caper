#!/usr/bin/env python3
"""Acquire Caper's licensed native Satoshi fonts from official Fontshare."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import tempfile
import urllib.request
import zipfile


DOWNLOAD_URL = "https://api.fontshare.com/v2/fonts/download/satoshi"
MAX_DOWNLOAD_BYTES = 8 * 1024 * 1024
MAX_RESOURCE_BYTES = 1024 * 1024
LICENSE_MEMBER = "Satoshi_Complete/License/FFL.txt"
LICENSE_SHA256 = "145e7fe2429a3336ba215c070ef722000e01348a3e1baaa127e871bb5012f554"
FONTS = {
    "Satoshi_Complete/Fonts/OTF/Satoshi-Regular.otf": (
        "Satoshi-Regular.otf",
        "711c6243cdc5431f9cc966e4de18bfb940365bad81acffd1e7948dbe3f254386",
    ),
    "Satoshi_Complete/Fonts/OTF/Satoshi-Medium.otf": (
        "Satoshi-Medium.otf",
        "93330866d109f6b2e298748958ec6fa4010cacef586783f281a0b268cab7fc6e",
    ),
    "Satoshi_Complete/Fonts/OTF/Satoshi-Bold.otf": (
        "Satoshi-Bold.otf",
        "50e4f9b7c1864c50761d729d6001bfac708c80457fa6fc41559a8ab1bd2573ff",
    ),
    "Satoshi_Complete/Fonts/OTF/Satoshi-Black.otf": (
        "Satoshi-Black.otf",
        "49bdb8b9436b8b9192f6f14b7ce4b96d1a3822e13c504c00c0b2842357d265cc",
    ),
}
CACHE_DIR = Path(__file__).resolve().parents[1] / "shared" / "fonts" / "cache"
ARCHIVE_PATH = CACHE_DIR / "Satoshi_Complete.zip"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require_hash(data: bytes, expected: str, label: str) -> None:
    actual = sha256(data)
    if actual != expected:
        raise RuntimeError(f"{label} SHA-256 mismatch: expected {expected}, got {actual}")


def download_archive(destination: Path = ARCHIVE_PATH) -> None:
    request = urllib.request.Request(
        DOWNLOAD_URL,
        headers={"User-Agent": "Caper native font acquisition/1.0"},
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            if response.url != DOWNLOAD_URL:
                raise RuntimeError(f"unexpected download redirect to {response.url}")
            with tempfile.NamedTemporaryFile(
                dir=destination.parent, prefix=".satoshi-", delete=False
            ) as output:
                temporary = Path(output.name)
                size = 0
                while chunk := response.read(64 * 1024):
                    size += len(chunk)
                    if size > MAX_DOWNLOAD_BYTES:
                        raise RuntimeError(
                            f"download exceeds {MAX_DOWNLOAD_BYTES} byte limit"
                        )
                    output.write(chunk)
        verified_resources(temporary)
        os.replace(temporary, destination)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def verified_resources(archive_path: Path) -> dict[str, tuple[bytes, str]]:
    # Fontshare's ZIP container varies between requests/regions. Authenticate
    # every byte actually embedded, including the license, rather than ZIP metadata.
    if archive_path.stat().st_size > MAX_DOWNLOAD_BYTES:
        raise RuntimeError("archive exceeds download size limit")
    resources = {**FONTS, LICENSE_MEMBER: ("Satoshi-FFL.txt", LICENSE_SHA256)}
    verified = {}
    with zipfile.ZipFile(archive_path) as archive:
        names = archive.namelist()
        missing = [name for name in resources if names.count(name) != 1]
        if missing:
            raise RuntimeError(
                "archive must contain exactly one of each required member; invalid: "
                + ", ".join(missing)
            )

        for member, (filename, expected_hash) in resources.items():
            if archive.getinfo(member).file_size > MAX_RESOURCE_BYTES:
                raise RuntimeError(f"{member} exceeds resource size limit")
            data = archive.read(member)
            require_hash(data, expected_hash, member)
            verified[filename] = (data, expected_hash)
    return verified


def extract_fonts(archive_path: Path = ARCHIVE_PATH, cache_dir: Path = CACHE_DIR) -> None:
    resources = verified_resources(archive_path)
    cache_dir.mkdir(parents=True, exist_ok=True)
    for filename, (data, expected_hash) in resources.items():
        destination = cache_dir / filename
        if destination.exists():
            require_hash(destination.read_bytes(), expected_hash, str(destination))
            continue
        with tempfile.NamedTemporaryFile(dir=cache_dir, prefix=f".{filename}-", delete=False) as output:
            temporary = Path(output.name)
            output.write(data)
        try:
            os.replace(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)


def main() -> None:
    if not ARCHIVE_PATH.exists():
        print(f"Downloading official Satoshi package from {DOWNLOAD_URL}")
        download_archive()
    extract_fonts()
    print(f"Verified Satoshi Regular, Medium, Bold, Black, and license in {CACHE_DIR}")


if __name__ == "__main__":
    main()
