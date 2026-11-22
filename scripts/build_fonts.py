#!/usr/bin/env python3
"""Build Caper's fonts: Inrond 1.0 (rounded Inter, SIL OFL) with roundness fixed at 40.

Writes the committed outputs; rerun only to change the release or the roundness:

    uv run --locked --group fonts python scripts/build_fonts.py

- apps/web/public/fonts/inrond-40/: woff2 subsets, still variable in weight and
  optical size, roman and italic, plus apps/web/src/fonts.css to load them.
- shared/fonts/: static Regular/Medium/Bold/Black TTFs (text optical size) and the
  license, embedded by every native client.
"""

from __future__ import annotations

import hashlib
import io
from pathlib import Path
import re
import tempfile
import urllib.request
import zipfile

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

RELEASE_URL = "https://github.com/Hairline-UI/inrond/releases/download/v1.0/Inrond-1.0.zip"
RELEASE_SHA256 = "f87751f79d83bfe0759b6b60629bb482bb002a35c3ac82e0f6ea019bcb8e88cd"
ROUNDNESS = 40
FAMILY = f"Inrond {ROUNDNESS}"
POSTSCRIPT = f"Inrond{ROUNDNESS}"
# The weights the clients use, and Inter's text optical size for the static files.
STATIC_WEIGHTS = {400: "Regular", 500: "Medium", 700: "Bold", 900: "Black"}
STATIC_OPSZ = 14

ROOT = Path(__file__).resolve().parents[1]
WEB_DIR = ROOT / "apps" / "web" / "public" / "fonts" / "inrond-40"
WEB_CSS = ROOT / "apps" / "web" / "src" / "fonts.css"
NATIVE_DIR = ROOT / "shared" / "fonts"
SOURCE = "Inrond-1.0/fonts/variable/Inrond[ROND,opsz,wght].ttf"
SOURCE_ITALIC = "Inrond-1.0/fonts/variable/Inrond-Italic[ROND,opsz,wght].ttf"
# Unicode-range subsets, read from the release's own stylesheet so they match upstream.
SUBSET_CSS = "Inrond-1.0/fonts/webfonts/inrond.css"


def download() -> zipfile.ZipFile:
    with urllib.request.urlopen(RELEASE_URL, timeout=60) as response:
        data = response.read()
    actual = hashlib.sha256(data).hexdigest()
    if actual != RELEASE_SHA256:
        raise RuntimeError(f"Inrond release SHA-256 mismatch: expected {RELEASE_SHA256}, got {actual}")
    return zipfile.ZipFile(io.BytesIO(data))


def subsets(release: zipfile.ZipFile) -> dict[str, str]:
    css = release.read(SUBSET_CSS).decode()
    found = {}
    for face in re.findall(r"@font-face\s*{[^}]*}", css):
        if "font-style: normal" not in face:
            continue
        name = re.search(r"Inrond\.([a-z-]+)\.woff2", face)
        ranges = re.search(r"unicode-range:\s*([^;]+);", face)
        if name and ranges:
            found[name.group(1)] = ranges.group(1).strip()
    if "latin" not in found:
        raise RuntimeError("Inrond stylesheet has no latin subset")
    return found


def rounded(release: zipfile.ZipFile, member: str, axes: dict[str, float]) -> TTFont:
    font = TTFont(io.BytesIO(release.read(member)))
    return instancer.instantiateVariableFont(font, {"ROND": ROUNDNESS, **axes})


def rename(font: TTFont, style: str, ribbi: bool = True) -> None:
    """Name a face of FAMILY; non-RIBBI weights get their own legacy family, as in Inter's static fonts."""
    names = font["name"]
    names.names = [record for record in names.names if record.nameID not in (1, 2, 3, 4, 6, 16, 17, 25)]
    postscript = f"{POSTSCRIPT}-{style}"
    for platform in ((3, 1, 0x409), (1, 0, 0)):
        names.setName(FAMILY if ribbi else f"{FAMILY} {style}", 1, *platform)
        names.setName(style if ribbi else "Regular", 2, *platform)
        names.setName(f"{postscript};Inrond 1.0", 3, *platform)
        names.setName(FAMILY if style == "Regular" else f"{FAMILY} {style}", 4, *platform)
        names.setName(postscript, 6, *platform)
        if not ribbi:
            names.setName(FAMILY, 16, *platform)
            names.setName(style, 17, *platform)


def build_web(release: zipfile.ZipFile, ranges: dict[str, str]) -> None:
    WEB_DIR.mkdir(parents=True, exist_ok=True)
    for path in WEB_DIR.glob("*.woff2"):
        path.unlink()
    faces = []
    for member, style, suffix in ((SOURCE, "normal", ""), (SOURCE_ITALIC, "italic", "-Italic")):
        font = rounded(release, member, {})
        rename(font, "Italic" if suffix else "Regular")
        for subset_name, unicode_range in ranges.items():
            options = subset.Options()
            options.flavor = "woff2"
            options.layout_features = ["*"]
            options.name_IDs = ["*"]
            options.notdef_outline = True
            subsetter = subset.Subsetter(options)
            subsetter.populate(unicodes=parse_ranges(unicode_range))
            part = TTFont(io.BytesIO(save(font)))
            subsetter.subset(part)
            filename = f"{POSTSCRIPT}{suffix}.{subset_name}.woff2"
            part.flavor = "woff2"
            part.save(WEB_DIR / filename)
            faces.append(
                "@font-face {\n"
                f'  font-family: "{FAMILY}";\n'
                f"  font-style: {style};\n"
                "  font-weight: 100 900;\n"
                "  font-display: swap;\n"
                f'  src: url("/fonts/inrond-40/{filename}") format("woff2");\n'
                f"  unicode-range: {unicode_range};\n"
                "}\n"
            )
    header = (
        "/* Generated by scripts/build_fonts.py: Inrond 1.0 at ROND 40, variable in weight and optical size. */\n\n"
    )
    WEB_CSS.write_text(header + "\n".join(faces))


def build_native(release: zipfile.ZipFile) -> None:
    for weight, style in STATIC_WEIGHTS.items():
        font = rounded(release, SOURCE, {"wght": weight, "opsz": STATIC_OPSZ})
        rename(font, style, ribbi=style in ("Regular", "Bold"))
        font["OS/2"].usWeightClass = weight
        bold = style == "Bold"
        font["OS/2"].fsSelection = (font["OS/2"].fsSelection & ~0b1100001) | (0b100000 if bold else 0b1000000)
        font["head"].macStyle = 1 if bold else 0
        font.save(NATIVE_DIR / f"{POSTSCRIPT}-{style}.ttf")
    (NATIVE_DIR / "OFL.txt").write_bytes(release.read("Inrond-1.0/OFL.txt"))


def parse_ranges(value: str) -> list[int]:
    unicodes = []
    for item in value.split(","):
        item = item.strip().removeprefix("U+")
        start, _, end = item.partition("-")
        unicodes.extend(range(int(start, 16), int(end or start, 16) + 1))
    return unicodes


def save(font: TTFont) -> bytes:
    with tempfile.SpooledTemporaryFile() as output:
        font.save(output)
        output.seek(0)
        return output.read()


def main() -> None:
    release = download()
    build_web(release, subsets(release))
    build_native(release)
    print(
        f"Built {FAMILY} into {WEB_DIR.relative_to(ROOT)}, {WEB_CSS.relative_to(ROOT)} and {NATIVE_DIR.relative_to(ROOT)}"
    )


if __name__ == "__main__":
    main()
