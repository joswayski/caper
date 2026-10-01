# /// script
# requires-python = ">=3.11"
# dependencies = ["vtracer==0.6.15"]
# ///
"""Author vector masters from the original 256px artwork, never the 64px atlas.

Run: uv run scripts/vectorize-avatars.py
Requires ImageMagick 7. Offline authoring only; no runtime/build dependency.
Then run node scripts/generate-avatar-vectors.mjs to export client resources.
The persisted mapping stays character = id % 100, colorway = floor(id / 100).
"""
from pathlib import Path
import subprocess
import tempfile
import xml.etree.ElementTree as ET

import vtracer

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "assets/avatars/vector-v2"
OUTPUT.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="caper-vector-") as scratch:
    for character in range(100):
        sheet = (ROOT / "apps/web/public/images/demo-avatars.webp" if character < 4
                 else ROOT / f"assets/avatars/companions-{character // 4:02}.webp")
        quadrant = character % 4
        png = str(Path(scratch) / "source.png")
        svg = str(Path(scratch) / "traced.svg")
        subprocess.run([
            "magick", str(sheet), "-crop",
            f"256x256+{quadrant % 2 * 256}+{quadrant // 2 * 256}",
            "+repage", png,
        ], check=True)
        vtracer.convert_image_to_svg_py(
            png, svg, colormode="color", hierarchical="stacked", mode="spline",
            filter_speckle=8, color_precision=4, layer_difference=24,
            corner_threshold=90, length_threshold=4, splice_threshold=45,
            path_precision=2,
        )
        paths = []
        for path in ET.parse(svg).getroot():
            assert path.tag == "{http://www.w3.org/2000/svg}path"
            paths.append(f'<path d="{path.attrib["d"].strip()}" fill="{path.attrib["fill"]}" transform="{path.attrib["transform"]}"/>')
        (OUTPUT / f"{character}.svg").write_text(
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">\n'
            + "\n".join(paths) + "\n</svg>\n"
        )
print("Traced 100 vector masters. Review them before publishing; tracing simplifies raster detail.")
