import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location("native_update_patches", SCRIPTS / "native_update_patches.py")
patches = importlib.util.module_from_spec(spec)
spec.loader.exec_module(patches)

# Stands in for `caper-updater delta`: patch size comes from the base's contents,
# and a base holding "broken" fails like a patch that does not rebuild the target.
FAKE_UPDATER = """#!/usr/bin/env python3
import json, sys
args = dict(zip(sys.argv[2::2], sys.argv[3::2]))
base = open(args["--base"]).read()
if base == "broken":
    sys.exit("patch does not rebuild the target")
size = int(base)
open(args["--output"], "wb").write(b"p" * size)
print(json.dumps({"size": size, "sha256": "a" * 64, "copied": 1, "patched": 1, "full": 0}))
"""


class UpdatePatchTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.updater = self.root / "caper-updater"
        self.updater.write_text(FAKE_UPDATER)
        self.updater.chmod(0o755)
        self.bases = self.root / "bases"
        self.bases.mkdir()
        self.archive = self.root / "Caper-Linux-x64.tar.gz"
        self.archive.write_bytes(b"x" * 1000)

    def base(self, build, contents, asset="Caper-Linux-x64.tar.gz"):
        (self.bases / f"{build}-{asset}").write_text(contents)

    def test_patches_come_from_the_newest_retained_earlier_builds_that_save_enough(self):
        for build in range(1, 10):
            self.base(build, "100")
        self.base(9, "600")  # Not worth it: more than half the full archive.
        self.base(8, "broken")
        self.base(10, "10")  # This build (a rerun) and newer ones never count.
        self.base(11, "10")
        self.base(7, "5", asset="Caper-Windows-x64.zip")
        (self.bases / "notes.txt").write_text("1")
        output = self.root / "out"
        made = patches.generate(
            "linux-x64", self.archive, 10, self.bases, self.updater, output, keep=patches.RETAINED_BASES
        )
        # Six retained bases (9..4); 9 is too large and 8 failed, so 7..4 remain.
        self.assertEqual([patch["from_build"] for patch in made], [7, 6, 5, 4])
        self.assertEqual(
            sorted(path.name for path in output.iterdir()),
            [f"Caper-Linux-x64-{build}-to-10.caperpatch" for build in (4, 5, 6, 7)] + ["patches-linux-x64.json"],
        )
        self.assertEqual(json.loads((output / "patches-linux-x64.json").read_text()), made)
        self.assertEqual(
            made[0], {"from_build": 7, "file": "Caper-Linux-x64-7-to-10.caperpatch", "size": 100, "sha256": "a" * 64}
        )

    def test_no_bases_means_no_patches_and_the_full_archive_alone(self):
        made = patches.generate("linux-x64", self.archive, 3, self.root / "missing", self.updater, self.root / "out")
        self.assertEqual(made, [])
        manifest = patches.add({"platforms": {}}, "linux-x64", self.archive, "https://example.com/r", made)
        self.assertEqual(
            manifest["platforms"]["linux-x64"],
            {
                "url": "https://example.com/r/Caper-Linux-x64.tar.gz",
                "sha256": patches.sha256(self.archive),
                "size": 1000,
            },
        )

    def test_listing_patches_keeps_the_signed_manifest_within_its_limit(self):
        made = [{"from_build": build, "file": f"p{build}", "size": 9, "sha256": "b" * 64} for build in (4, 7, 5)]
        notes = "• " + "x" * 2000
        manifest = {
            "schema": 1,
            "build": 8,
            "platforms": {},
            "changelog": [{"build": build, "version": f"0.1.{build}", "notes": notes} for build in range(8, 0, -1)],
            "changelog_from_build": 1,
        }
        for platform in ("linux-x64", "windows-x64"):
            manifest = patches.add(manifest, platform, self.archive, "https://example.com/r", made)
        self.assertEqual([patch["from_build"] for patch in manifest["platforms"]["linux-x64"]["patches"]], [7, 5, 4])
        self.assertEqual(manifest["platforms"]["linux-x64"]["patches"][0]["url"], "https://example.com/r/p7")
        self.assertLessEqual(len(patches.encode(manifest)), 64 * 1024)
        self.assertEqual(len(manifest["changelog"]), 8, "plenty of room: nothing trimmed")
        # When patches push it over the limit, the oldest notes go, not patches.
        manifest["changelog"] = [
            {"build": build, "version": f"0.1.{build}", "notes": notes} for build in range(40, 8, -1)
        ]
        manifest = patches.add(manifest, "macos-arm64", self.archive, "https://example.com/r", made)
        self.assertLessEqual(len(patches.encode(manifest)), 64 * 1024)
        self.assertEqual(len(manifest["platforms"]["macos-arm64"]["patches"]), 3)
        self.assertLess(len(manifest["changelog"]), 32)
        # Clients see the history as complete only from the oldest build kept.
        self.assertEqual(manifest["changelog_from_build"], manifest["changelog"][-1]["build"] - 1)

    def test_each_asset_keeps_its_newest_builds_and_other_files_stay(self):
        names = [f"{build}-Caper-Linux-x64.tar.gz" for build in (99, 100, 101, 102, 103, 104, 105, 9)]
        names += ["104-Caper-Windows-x64.zip", "README", "100-Caper-macOS-Intel.zip"]
        self.assertEqual(
            patches.stale(names, keep=6),
            ["99-Caper-Linux-x64.tar.gz", "9-Caper-Linux-x64.tar.gz"],
            "build numbers compare as numbers, per asset",
        )

    def test_patch_names_name_both_builds(self):
        self.assertEqual(
            patches.patch_name("Caper-macOS-Apple-Silicon.zip", 418, 420),
            "Caper-macOS-Apple-Silicon-418-to-420.caperpatch",
        )
        with self.assertRaises(ValueError):
            patches.stem("Caper-Windows-x64-Setup.exe")


if __name__ == "__main__":
    unittest.main()
