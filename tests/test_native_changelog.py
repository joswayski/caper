import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "native_changelog", Path(__file__).resolve().parents[1] / "scripts/native_changelog.py"
)
changelog = importlib.util.module_from_spec(spec)
spec.loader.exec_module(changelog)


class ChangelogTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.repo = Path(self.directory.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.test")

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "commit.gpgsign=false", "-C", str(self.repo), *args], text=True
        ).strip()

    def commit(self, subject):
        self.git("commit", "--allow-empty", "-qm", subject)
        return self.git("rev-parse", "HEAD")

    def latest(self, build, commit):
        return {
            "schema": 1,
            "build": build,
            "version": f"0.1.{build}",
            "commit": commit,
            "notes": "Legacy title",
            "platforms": {},
        }

    def produce(self, latest, previous=None):
        path = self.repo / "latest.json"
        path.write_text(json.dumps(latest))
        args = ["python3", spec.origin, str(path)]
        if previous:
            old = self.repo / "previous.json"
            old.write_text(json.dumps(previous))
            args += ["--previous", str(old)]
        subprocess.run(args, cwd=self.repo, check=True, capture_output=True)
        return json.loads(path.read_bytes())

    def test_all_commits_between_releases_and_skipped_build_history(self):
        first = self.commit("Initial feature (#1)")
        old = self.produce(self.latest(9, first))
        self.commit("Channel navigation (#2)")
        newest = self.commit("Audio device fix (#3)")
        current = self.produce(self.latest(12, newest), old)
        self.assertEqual(current["notes"], "• Audio device fix (#3)\n• Channel navigation (#2)")
        self.assertEqual([entry["build"] for entry in current["changelog"]], [12, 9])
        self.assertEqual(current["changelog"][1]["notes"], "• Initial feature (#1)")
        self.assertEqual(current["changelog_from_build"], 0)

    def test_legacy_bootstrap_and_rerun_do_not_invent_or_lose_notes(self):
        first = self.commit("Already installed")
        old = self.latest(18, first)
        newest = self.commit("New change")
        current = self.produce(self.latest(20, newest), old)
        self.assertEqual(current["changelog_from_build"], 18)
        self.assertEqual(current["changelog"], [{"build": 20, "version": "0.1.20", "notes": "• New change"}])
        self.assertEqual(self.produce(self.latest(20, newest), current), current)

    def test_rollback_and_same_source_rebuild_are_explicit(self):
        first = self.commit("Earlier source")
        newest = self.commit("Later source")
        old = self.produce(self.latest(5, newest))
        rollback = self.produce(self.latest(6, first), old)
        self.assertEqual(rollback["notes"], f"• Rebuilt earlier source {first[:7]}; replaces the previous release.")
        rebuilt = self.produce(self.latest(7, first), rollback)
        self.assertEqual(rebuilt["notes"], "• Rebuilt the same source; no new code changes.")

    def test_published_build_number_cannot_regress_or_change_source(self):
        first = self.commit("First")
        second = self.commit("Second")
        old = self.latest(5, first)
        with self.assertRaises(subprocess.CalledProcessError):
            self.produce(self.latest(4, second), old)
        with self.assertRaises(subprocess.CalledProcessError):
            self.produce(self.latest(5, second), old)

    def test_size_limit_keeps_whole_groups_and_advances_exact_cutoff(self):
        manifest = self.latest(12, "abc")
        manifest.update(
            changelog_from_build=0,
            changelog=[
                {"build": 12, "version": "0.1.12", "notes": "Newest"},
                {"build": 9, "version": "0.1.9", "notes": "Keep this"},
                {"build": 4, "version": "0.1.4", "notes": "ü" * 200},
            ],
        )
        limit = len(changelog.encode({**manifest, "changelog": manifest["changelog"][:2], "changelog_from_build": 4}))
        with patch.object(changelog, "MAX_MANIFEST_BYTES", limit):
            result = changelog.fit(manifest)
        self.assertEqual(len(changelog.encode(result)), limit)
        self.assertEqual([entry["build"] for entry in result["changelog"]], [12, 9])
        self.assertEqual(result["changelog_from_build"], 4)
        with patch.object(changelog, "MAX_MANIFEST_BYTES", 10), self.assertRaises(ValueError):
            changelog.fit(result)


if __name__ == "__main__":
    unittest.main()
