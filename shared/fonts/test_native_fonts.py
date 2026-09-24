#!/usr/bin/env python3

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile


sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "native_fonts.py"
SPEC = importlib.util.spec_from_file_location("native_fonts", SCRIPT)
assert SPEC and SPEC.loader
native_fonts = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native_fonts)


class NativeFontsTest(unittest.TestCase):
    def test_checksum_mismatch_is_rejected(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
            native_fonts.require_hash(b"tampered", "0" * 64, "fixture")

    def test_missing_archive_member_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive_path = Path(temporary) / "font.zip"
            with zipfile.ZipFile(archive_path, "w") as archive:
                archive.writestr(native_fonts.LICENSE_MEMBER, "license")

            with self.assertRaisesRegex(RuntimeError, "required member"):
                native_fonts.extract_fonts(archive_path, Path(temporary))

    def test_fonts_and_license_are_preserved_and_corrupt_cache_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive_path = root / "font.zip"
            font = b"unmodified font bytes"
            license_text = b"unmodified license\r\n"
            member = "Satoshi_Complete/Fonts/OTF/Test.otf"
            with zipfile.ZipFile(archive_path, "w") as archive:
                archive.writestr(member, font)
                archive.writestr(native_fonts.LICENSE_MEMBER, license_text)
                archive.writestr("../unexpected.txt", b"must not be extracted")
            with patch.object(native_fonts, "LICENSE_SHA256", native_fonts.sha256(license_text)), patch.object(
                native_fonts, "FONTS", {member: ("Test.otf", native_fonts.sha256(font))}
            ):
                cache = root / "cache"
                native_fonts.extract_fonts(archive_path, cache)
                self.assertEqual((cache / "Test.otf").read_bytes(), font)
                self.assertEqual((cache / "Satoshi-FFL.txt").read_bytes(), license_text)
                self.assertFalse((root / "unexpected.txt").exists())
                native_fonts.extract_fonts(archive_path, cache)
                (cache / "Satoshi-FFL.txt").write_bytes(b"changed license")
                with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
                    native_fonts.extract_fonts(archive_path, cache)

    def test_container_metadata_can_vary_but_resource_bytes_cannot(self) -> None:
        member = "Satoshi_Complete/Fonts/OTF/Test.otf"
        font, license_text = b"font", b"license"
        with tempfile.TemporaryDirectory() as temporary, patch.object(
            native_fonts, "FONTS", {member: ("Test.otf", native_fonts.sha256(font))}
        ), patch.object(native_fonts, "LICENSE_SHA256", native_fonts.sha256(license_text)):
            root = Path(temporary)
            hashes = []
            for year in (2020, 2026):
                path = root / f"{year}.zip"
                with zipfile.ZipFile(path, "w") as archive:
                    for name, data in ((member, font), (native_fonts.LICENSE_MEMBER, license_text)):
                        archive.writestr(zipfile.ZipInfo(name, (year, 1, 1, 0, 0, 0)), data)
                hashes.append(native_fonts.sha256(path.read_bytes()))
                native_fonts.extract_fonts(path, root / str(year))
                self.assertEqual((root / str(year) / "Test.otf").read_bytes(), font)
            self.assertNotEqual(*hashes)
            for changed_member in (member, native_fonts.LICENSE_MEMBER):
                path = root / "tampered.zip"
                with zipfile.ZipFile(path, "w") as archive:
                    for name, data in ((member, font), (native_fonts.LICENSE_MEMBER, license_text)):
                        archive.writestr(name, data + b"changed" if name == changed_member else data)
                with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
                    native_fonts.extract_fonts(path, root / "refused")
                self.assertFalse((root / "refused").exists())


if __name__ == "__main__":
    unittest.main()
