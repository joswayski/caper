#!/usr/bin/env python3
"""Build Caper's fonts: Nunito (display) and Figtree (UI/chat), both SIL OFL.

Writes the committed outputs; rerun only to update pinned upstream revisions:

    uv run --locked --group fonts python scripts/build_fonts.py

- apps/web/public/fonts/caper/: variable woff2 subsets (latin and latin-ext),
  plus apps/web/src/fonts.css to load them.
- shared/fonts/: static TTFs (Figtree Regular/Medium/Bold/Black and Nunito
  Bold/ExtraBold) and the combined SIL OFL notice, embedded by every native
  client.
"""

from __future__ import annotations

import hashlib
import io
from pathlib import Path
import tempfile
import urllib.request

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

GOOGLE_FONTS_COMMIT = "06bfe0d62a98375cb09c942fa18e68bd32cff057"
RAW_BASE = f"https://raw.githubusercontent.com/google/fonts/{GOOGLE_FONTS_COMMIT}"

UPSTREAM_FILES: dict[str, tuple[str, str]] = {
    "figtree_roman": (
        f"{RAW_BASE}/ofl/figtree/Figtree%5Bwght%5D.ttf",
        "26ad3db9b31ff7dde67a91ff515d022d2f495cd506590699cf264f0bfe6fb714",
    ),
    "figtree_italic": (
        f"{RAW_BASE}/ofl/figtree/Figtree-Italic%5Bwght%5D.ttf",
        "94dec1f18b9275d69e8b4a91b6514bdc18199048347e1c36e1285b11b0b87653",
    ),
    "figtree_ofl": (
        f"{RAW_BASE}/ofl/figtree/OFL.txt",
        "140d37233e7f3ce7313798befa9600893bcceaf41a55fa0fa5ad52f7f657a268",
    ),
    "nunito_roman": (
        f"{RAW_BASE}/ofl/nunito/Nunito%5Bwght%5D.ttf",
        "bb55a5ca5c2042335b3991af27c4d0705d0ef41cac6164ac737fd8f2a1e85207",
    ),
    "nunito_ofl": (
        f"{RAW_BASE}/ofl/nunito/OFL.txt",
        "580df76c95a1ec5ab878ceb25bb3d85c6a076804e9c970c8c6972aea775fdf65",
    ),
}

# Standard Google Fonts latin and latin-ext unicode ranges.
SUBSETS: dict[str, str] = {
    "latin": (
        "U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, "
        "U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, "
        "U+2212, U+2215, U+FEFF, U+FFFD"
    ),
    "latin-ext": (
        "U+0100-02BA, U+02BD-02C5, U+02C7-02CC, U+02CE-02D7, U+02DD-02FF, "
        "U+0304, U+0308, U+0329, U+1D00-1DBF, U+1E00-1E9F, U+1EF2-1EFF, "
        "U+2020, U+20A0-20AB, U+20AD-20C0, U+2113, U+2C60-2C7F, U+A720-A7FF"
    ),
}

FIGTREE_WEIGHTS = {400: "Regular", 500: "Medium", 700: "Bold", 900: "Black"}
NUNITO_WEIGHTS = {700: "Bold", 800: "ExtraBold"}

ROOT = Path(__file__).resolve().parents[1]
WEB_DIR = ROOT / "apps" / "web" / "public" / "fonts" / "caper"
LEGACY_WEB_DIR = ROOT / "apps" / "web" / "public" / "fonts" / "inrond-40"
WEB_CSS = ROOT / "apps" / "web" / "src" / "fonts.css"
NATIVE_DIR = ROOT / "shared" / "fonts"


def fetch(key: str) -> bytes:
    url, expected_sha = UPSTREAM_FILES[key]
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read()
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected_sha:
        raise RuntimeError(f"{key} SHA-256 mismatch: expected {expected_sha}, got {actual}")
    return data


def rename(font: TTFont, family: str, postscript_prefix: str, style: str, ribbi: bool = True) -> None:
    """Assign predictable cross-platform family and PostScript names."""
    names = font["name"]
    names.names = [record for record in names.names if record.nameID not in (1, 2, 3, 4, 6, 16, 17, 25)]
    postscript = f"{postscript_prefix}-{style}"
    for platform in ((3, 1, 0x409), (1, 0, 0)):
        names.setName(family if ribbi else f"{family} {style}", 1, *platform)
        names.setName(style if ribbi else "Regular", 2, *platform)
        names.setName(f"{postscript};Caper", 3, *platform)
        names.setName(family if style == "Regular" else f"{family} {style}", 4, *platform)
        names.setName(postscript, 6, *platform)
        if not ribbi:
            names.setName(family, 16, *platform)
            names.setName(style, 17, *platform)


def parse_ranges(value: str) -> list[int]:
    unicodes = []
    for item in value.split(","):
        item = item.strip().removeprefix("U+")
        start, _, end = item.partition("-")
        unicodes.extend(range(int(start, 16), int(end or start, 16) + 1))
    return unicodes


def save_bytes(font: TTFont) -> bytes:
    with tempfile.SpooledTemporaryFile() as output:
        font.save(output)
        output.seek(0)
        return output.read()


def emit_web_family(
    raw_ttf: bytes,
    family: str,
    file_prefix: str,
    style: str,
    weight_range: str,
    axes: dict[str, float],
    faces: list[str],
) -> None:
    font = TTFont(io.BytesIO(raw_ttf))
    if axes:
        font = instancer.instantiateVariableFont(font, axes)
    font_bytes = save_bytes(font)
    for subset_name, unicode_range in SUBSETS.items():
        options = subset.Options()
        options.flavor = "woff2"
        options.layout_features = ["*"]
        options.name_IDs = ["*"]
        options.notdef_outline = True
        subsetter = subset.Subsetter(options)
        subsetter.populate(unicodes=parse_ranges(unicode_range))
        part = TTFont(io.BytesIO(font_bytes))
        subsetter.subset(part)
        filename = f"{file_prefix}.{subset_name}.woff2"
        part.flavor = "woff2"
        part.save(WEB_DIR / filename)
        faces.append(
            "@font-face {\n"
            f'  font-family: "{family}";\n'
            f"  font-style: {style};\n"
            f"  font-weight: {weight_range};\n"
            "  font-display: swap;\n"
            f'  src: url("/fonts/caper/{filename}") format("woff2");\n'
            f"  unicode-range: {unicode_range};\n"
            "}\n"
        )


def build_web(figtree_roman: bytes, figtree_italic: bytes, nunito_roman: bytes) -> None:
    WEB_DIR.mkdir(parents=True, exist_ok=True)
    for path in WEB_DIR.glob("*.woff2"):
        path.unlink()
    if LEGACY_WEB_DIR.exists():
        for path in LEGACY_WEB_DIR.glob("*.woff2"):
            path.unlink()
        try:
            LEGACY_WEB_DIR.rmdir()
        except OSError:
            pass

    faces: list[str] = []
    emit_web_family(figtree_roman, "Figtree", "Figtree", "normal", "300 900", {}, faces)
    emit_web_family(figtree_italic, "Figtree", "Figtree-Italic", "italic", "300 900", {}, faces)
    emit_web_family(
        nunito_roman,
        "Nunito",
        "Nunito",
        "normal",
        "400 900",
        {},
        faces,
    )

    header = "/* Generated by scripts/build_fonts.py: Figtree (UI/chat) and Nunito (display), SIL OFL. */\n\n"
    WEB_CSS.write_text(header + "\n".join(faces))


def emit_static(
    raw_ttf: bytes,
    family: str,
    postscript_prefix: str,
    weights: dict[int, str],
    fixed_axes: dict[str, float],
) -> None:
    for weight, style in weights.items():
        font = TTFont(io.BytesIO(raw_ttf))
        font = instancer.instantiateVariableFont(font, {**fixed_axes, "wght": weight})
        rename(font, family, postscript_prefix, style, ribbi=style in ("Regular", "Bold"))
        font["OS/2"].usWeightClass = weight
        bold = style == "Bold"
        font["OS/2"].fsSelection = (font["OS/2"].fsSelection & ~0b1100001) | (0b100000 if bold else 0b1000000)
        font["head"].macStyle = 1 if bold else 0
        font.save(NATIVE_DIR / f"{postscript_prefix}-{style}.ttf")


def build_native(figtree_roman: bytes, nunito_roman: bytes, figtree_ofl: bytes, nunito_ofl: bytes) -> None:
    NATIVE_DIR.mkdir(parents=True, exist_ok=True)
    for pattern in ("Inrond40-*.ttf", "BricolageGrotesque-*.ttf"):
        for old in NATIVE_DIR.glob(pattern):
            old.unlink()
    emit_static(figtree_roman, "Figtree", "Figtree", FIGTREE_WEIGHTS, {})
    emit_static(
        nunito_roman,
        "Nunito",
        "Nunito",
        NUNITO_WEIGHTS,
        {},
    )
    combined_license = (
        "Figtree — SIL Open Font License 1.1\n"
        "===================================\n\n"
        + figtree_ofl.decode("utf-8").strip()
        + "\n\n\n"
        + "Nunito — SIL Open Font License 1.1\n"
        + "==================================\n\n"
        + nunito_ofl.decode("utf-8").strip()
        + "\n"
    )
    (NATIVE_DIR / "OFL.txt").write_text(combined_license, encoding="utf-8")


def main() -> None:
    figtree_roman = fetch("figtree_roman")
    figtree_italic = fetch("figtree_italic")
    figtree_ofl = fetch("figtree_ofl")
    nunito_roman = fetch("nunito_roman")
    nunito_ofl = fetch("nunito_ofl")
    build_web(figtree_roman, figtree_italic, nunito_roman)
    build_native(figtree_roman, nunito_roman, figtree_ofl, nunito_ofl)
    print(
        f"Built Figtree + Nunito into {WEB_DIR.relative_to(ROOT)}, "
        f"{WEB_CSS.relative_to(ROOT)} and {NATIVE_DIR.relative_to(ROOT)}"
    )


if __name__ == "__main__":
    main()
