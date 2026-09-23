import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "native_artifacts", Path(__file__).resolve().parents[1] / "scripts/native_artifacts.py"
)
artifacts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(artifacts)


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.destination = self.root / "output"
        self.revision = "1234567890abcdef" * 2 + "12345678"

    def stage(self, target="windows-x64"):
        artifacts.stage(target, self.revision, self.source, self.destination)

    def test_exact_packages_provenance_and_known_checksum(self):
        (self.source / "Caper-windows-x64.zip").write_bytes(b"abc")
        (self.source / "private-key.txt").write_text("must not be uploaded")
        self.stage()
        manifest = json.loads((self.destination / "BUILD.json").read_text())
        digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        self.assertEqual(manifest["revision"], self.revision)
        self.assertEqual(manifest["distribution"], "not-distribution-signed")
        self.assertFalse(manifest["featureParityVerified"])
        self.assertFalse(manifest["nativeVoiceVerified"])
        self.assertEqual(manifest["packages"], [
            {"name": "Caper-windows-x64.zip", "bytes": 3, "sha256": digest}
        ])
        self.assertEqual((self.destination / "SHA256SUMS").read_text(),
                         f"{digest}  Caper-windows-x64.zip\n")
        self.assertEqual({p.name for p in self.destination.iterdir()},
                         {"Caper-windows-x64.zip", "BUILD.json", "SHA256SUMS"})
        self.assertEqual((self.destination / "Caper-windows-x64.zip").read_bytes(), b"abc")

    def test_linux_requires_both_packages_before_staging(self):
        (self.source / "Caper-linux-x64.tar.gz").write_bytes(b"archive")
        with self.assertRaises(ValueError):
            self.stage("linux-x64")
        self.assertFalse(self.destination.exists())

    def test_empty_and_symlink_packages_rejected(self):
        package = self.source / "Caper-windows-x64.zip"
        package.touch()
        with self.assertRaises(ValueError):
            self.stage()
        package.unlink()
        private = self.root / "private"
        private.write_bytes(b"not a package")
        package.symlink_to(private)
        with self.assertRaises(ValueError):
            self.stage()

    def test_stale_destination_rejected(self):
        (self.source / "Caper-windows-x64.zip").write_bytes(b"abc")
        self.destination.mkdir()
        with self.assertRaises(FileExistsError):
            self.stage()

    def test_mobile_distribution_labels_are_not_interchangeable(self):
        for target, distribution in [
            ("ios-simulator-arm64", "simulator-only"), ("android-debug", "debug-signed")
        ]:
            with self.subTest(target=target):
                name, = artifacts.PACKAGES[target]
                (self.source / name).write_bytes(b"abc")
                output = self.root / target
                artifacts.stage(target, self.revision, self.source, output)
                self.assertEqual(json.loads((output / "BUILD.json").read_text())["distribution"], distribution)

    def test_invalid_revision_rejected_before_writes(self):
        self.revision = "main"
        with self.assertRaises(ValueError):
            self.stage()
        self.assertFalse(self.destination.exists())


if __name__ == "__main__":
    unittest.main()
