import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest


@unittest.skipUnless(shutil.which("gio") and shutil.which("desktop-file-validate"), "needs GIO desktop tools")
class LinuxInstallTests(unittest.TestCase):
    def test_launcher_runs_from_a_prefix_with_spaces_and_reserved_characters(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stage = root / "stage"
            for name in ("lib", "share", "licenses"):
                (stage / name).mkdir(parents=True)
            installer = Path(__file__).resolve().parents[1] / "apps/native/linux/install.sh"
            shutil.copy(installer, stage / "install.sh")
            (stage / "lib/caper-linux").write_text('#!/bin/sh\nprintf launched > "$LAUNCH_MARKER"\n')
            for name in ("ffmpeg", "libonnxruntime.so.1.23.2"):
                (stage / "lib" / name).touch()
            (stage / "share/chat.caper.linux.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg"/>')
            (stage / "share/chat.caper.linux.desktop").write_text(
                "[Desktop Entry]\nType=Application\nName=Caper test\nExec=caper-linux\n"
            )
            (stage / "REVISION").write_text("fixture\n")
            (stage / "licenses/NOTICE").write_text("fixture notice\n")
            for name in ("normal", 'spaces $`"\\&|% prefix'):
                with self.subTest(prefix=name):
                    prefix = root / name
                    marker = root / "launched"
                    marker.unlink(missing_ok=True)
                    env = {**os.environ, "PREFIX": str(prefix), "LAUNCH_MARKER": str(marker)}
                    command = ["bash", str(stage / "install.sh")]
                    # A reinstall must preserve the working launch path and notices.
                    for _ in range(2):
                        subprocess.run(command, env=env, check=True, capture_output=True)
                    desktop = prefix / "share/applications/chat.caper.linux.desktop"
                    subprocess.run(["desktop-file-validate", str(desktop)], check=True, capture_output=True)
                    subprocess.run(["gio", "launch", str(desktop)], env=env, check=True, capture_output=True)
                    for _ in range(100):
                        if marker.exists():
                            break
                        time.sleep(0.02)
                    self.assertEqual(marker.read_text(), "launched")
                    self.assertEqual((prefix / "lib/caper-linux/licenses/NOTICE").read_text(), "fixture notice\n")
                    subprocess.run([*command, "--uninstall"], env=env, check=True, capture_output=True)
                    self.assertFalse(desktop.exists())
                    self.assertFalse((prefix / "lib/caper-linux").exists())
                    self.assertFalse((prefix / "bin/caper-linux").is_symlink())


if __name__ == "__main__":
    unittest.main()
