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
ARCHIVE_SHA256 = "5c0469e84d3331424a21adde6954dacfc6d9f080c204026efd53c2f40e8dc1c3"
MAX_DOWNLOAD_BYTES = 8 * 1024 * 1024
LICENSE_MEMBER = "Satoshi_Complete/License/FFL.txt"
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
ARCHIVE_PATH = CACHE_DIR / f"Satoshi_Complete-{ARCHIVE_SHA256}.zip"


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
                digest = hashlib.sha256()
                size = 0
                while chunk := response.read(64 * 1024):
                    size += len(chunk)
                    if size > MAX_DOWNLOAD_BYTES:
                        raise RuntimeError(
                            f"download exceeds {MAX_DOWNLOAD_BYTES} byte limit"
                        )
                    digest.update(chunk)
                    output.write(chunk)
        actual = digest.hexdigest()
        if actual != ARCHIVE_SHA256:
            raise RuntimeError(
                f"archive SHA-256 mismatch: expected {ARCHIVE_SHA256}, got {actual}"
            )
        os.replace(temporary, destination)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def extract_fonts(archive_path: Path = ARCHIVE_PATH, cache_dir: Path = CACHE_DIR) -> None:
    archive_data = archive_path.read_bytes()
    require_hash(archive_data, ARCHIVE_SHA256, "archive")

    with zipfile.ZipFile(archive_path) as archive:
        names = archive.namelist()
        required = [*FONTS, LICENSE_MEMBER]
        missing = [name for name in required if names.count(name) != 1]
        if missing:
            raise RuntimeError(
                "archive must contain exactly one of each required member; invalid: "
                + ", ".join(missing)
            )

        # The pinned archive authenticates the accompanying license too. Keep it
        # unmodified beside the fonts so each application can bundle the terms.
        resources = {
            **FONTS,
            LICENSE_MEMBER: ("Satoshi-FFL.txt", sha256(archive.read(LICENSE_MEMBER))),
        }
        cache_dir.mkdir(parents=True, exist_ok=True)
        for member, (filename, expected_hash) in resources.items():
            data = archive.read(member)
            require_hash(data, expected_hash, member)
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
    if ARCHIVE_PATH.exists():
        require_hash(ARCHIVE_PATH.read_bytes(), ARCHIVE_SHA256, str(ARCHIVE_PATH))
    else:
        print(f"Downloading official Satoshi package from {DOWNLOAD_URL}")
        download_archive()
    extract_fonts()
    print(f"Verified Satoshi Regular, Medium, Bold, Black, and license in {CACHE_DIR}")


if __name__ == "__main__":
    main()
