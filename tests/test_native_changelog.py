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

    def commit(self, subject, files=None, author=None):
        if files is None:
            files = {"apps/native/desktop/src/main.rs": subject + "\n"}
        for name, contents in files.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            if contents is None:
                path.unlink()
            else:
                path.write_text(contents)
        self.git("add", "-A")
        self.git("commit", "--allow-empty", "-qm", subject, *(["--author", author] if author else []))
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

    def test_bootstrap_and_incremental_notes_exclude_internal_changes(self):
        first = self.commit("Initial feature (#1)")
        old = self.produce(self.latest(9, first))
        self.commit("Fix native UI checks (#2)", {"apps/native/apple/Tests/CaperCoreTests/ModelTests.swift": "test"})
        self.commit("Keep jobs queued for self-hosted machines (#3)", {".github/workflows/native.yml": "ci"})
        self.commit("Deploy gateway replicas (#4)", {"compose/gateway.yaml": "infra"})
        self.commit(
            "Make store-push-credentials.sh work on macOS (#5)", {"scripts/store-push-credentials.sh": "script"}
        )
        self.commit("Update release publishing dependencies (#6)", {"pyproject.toml": "tooling", "uv.lock": "lock"})
        self.commit("Clarify setup (#7)", {"README.md": "docs"})
        self.commit("Bump runtime dependency (#8)", {"apps/api/src/lib.rs": "dependency"})
        self.commit("Refresh runtime dependency (#9)", author="dependabot[bot] <bot@example.test>")
        self.commit("Upgrade JS tooling to Vite, Vitest and Oxc (#10)")
        self.commit("Fix desktop DM avatar test and API clippy (#11)")
        self.commit("Fix message editor limit feedback and browser regression coverage (#12)")
        self.commit("Fix macOS CI window isolation and message action hit targets (#14)")
        newest = self.commit("Fix stale queued push delivery (#13)", {"apps/api/src/push/delivery.rs": "fix"})
        expected = (
            "• Fix stale queued push delivery (#13)\n"
            "• Fix macOS CI window isolation and message action hit targets (#14)\n"
            "• Fix message editor limit feedback and browser regression coverage (#12)"
        )
        current = self.produce(self.latest(12, newest), old)
        self.assertEqual(current["notes"], expected)
        self.assertEqual(self.produce(self.latest(12, newest))["notes"], expected + "\n• Initial feature (#1)")

    def test_test_paths_do_not_hide_mixed_product_changes(self):
        first = self.commit("Initial feature")
        old = self.produce(self.latest(1, first))
        self.commit(
            "Improve regression coverage",
            {
                "apps/web/src/server/chat.test.ts": "test",
                "apps/web/src/chat/Chat.spec.tsx": "test",
                "apps/api/src/push/tests.rs": "test",
                "apps/api/src/direct/tests/recovery.rs": "test",
                "apps/native/android/app/src/test/java/ModelTest.kt": "test",
                "apps/native/android/app/src/androidTest/java/MenuUiTest.kt": "test",
                "shared/fonts/test_native_fonts.py": "test",
                "tests/test_native_changelog.py": "test",
            },
        )
        newest = self.commit(
            "Add microphone test playback",
            {"apps/web/src/pages/MicPlayback.tsx": "feature", "apps/web/src/pages/MicPlayback.test.tsx": "test"},
        )
        self.assertEqual(self.produce(self.latest(2, newest), old)["notes"], "• Add microphone test playback")

    def test_product_assets_migrations_and_deletions_are_changes(self):
        first = self.commit("Initial feature")
        old = self.produce(self.latest(1, first))
        self.commit("Align controls", {"shared/design.css": "styles"})
        self.commit("Add notification settings", {"apps/api/migrations/notifications.sql": "migration"})
        self.commit("Improve launcher icon", {"apps/native/android/app/src/main/res/drawable/icon.xml": "icon"})
        self.commit("Improve emoji artwork", {"apps/web/public/emoji/café.svg": "artwork"})
        newest = self.commit("Remove obsolete desktop screen", {"apps/native/desktop/src/main.rs": None})
        self.assertEqual(
            self.produce(self.latest(2, newest), old)["notes"],
            "• Remove obsolete desktop screen\n• Improve emoji artwork\n• Improve launcher icon\n"
            "• Add notification settings\n• Align controls",
        )

    def test_merge_notes_use_only_first_parent_product_changes(self):
        first = self.commit("Initial feature")
        old = self.produce(self.latest(1, first))
        self.git("checkout", "-qb", "feature")
        self.commit("Implementation detail", {"apps/web/src/chat/Chat.tsx": "feature"})
        self.git("checkout", "-q", "-")
        self.git("merge", "--no-ff", "-qm", "Add channel navigation (#2)", "feature")
        self.git("checkout", "-qb", "checks")
        self.commit("Test implementation detail", {"apps/web/src/chat/Chat.test.tsx": "test"})
        self.git("checkout", "-q", "-")
        self.git("merge", "--no-ff", "-qm", "Improve navigation checks (#3)", "checks")
        newest = self.git("rev-parse", "HEAD")
        self.assertEqual(self.produce(self.latest(2, newest), old)["notes"], "• Add channel navigation (#2)")

    def test_retained_history_and_reruns_clean_old_unfiltered_notes(self):
        self.commit("Fix user-facing delivery (#1)", {"apps/api/src/push/delivery.rs": "fix"})
        internal = self.commit("Improve delivery checks (#2)", {"apps/api/src/push/tests.rs": "test"})
        old = self.latest(9, internal)
        old.update(
            notes="• Improve delivery checks (#2)\n• Fix user-facing delivery (#1)",
            changelog_from_build=0,
            changelog=[
                {
                    "build": 9,
                    "version": "0.1.9",
                    "notes": "• Improve delivery checks (#2)\n• Fix user-facing delivery (#1)",
                }
            ],
        )
        rerun = self.produce(self.latest(9, internal), old)
        self.assertEqual(rerun["notes"], "• Fix user-facing delivery (#1)")
        self.assertEqual(rerun["changelog"][0]["notes"], rerun["notes"])
        self.assertEqual(self.produce(self.latest(9, internal), rerun), rerun)
        newest = self.commit("Fix sign-in recovery (#3)")
        current = self.produce(self.latest(12, newest), old)
        self.assertEqual(current["changelog"][1]["notes"], "• Fix user-facing delivery (#1)")
        self.assertEqual([entry["build"] for entry in current["changelog"]], [12, 9])
        self.assertEqual(current["changelog_from_build"], 0)

    def test_retained_unknown_empty_and_ambiguous_notes_are_preserved(self):
        self.commit("Improve delivery", {"apps/api/src/push/tests.rs": "test"})
        newest = self.commit("Improve delivery", {"apps/api/src/push/delivery.rs": "fix"})
        old = self.latest(9, newest)
        old.update(
            notes="• Improve delivery\n• Custom release summary\nAdditional context.",
            changelog_from_build=0,
            changelog=[{"build": 9, "version": "0.1.9", "notes": ""}],
        )
        self.assertEqual(self.produce(self.latest(9, newest), old), old)

    def test_internal_only_build_is_not_reported_as_identical_source(self):
        first = self.commit("Initial feature")
        old = self.produce(self.latest(1, first))
        newest = self.commit("Improve CI", {".github/workflows/native.yml": "ci"})
        current = self.produce(self.latest(2, newest), old)
        self.assertEqual(current["notes"], "• No user-facing changes in this build.")
        self.assertEqual(current["changelog"][0]["notes"], current["notes"])
        self.assertEqual(current["changelog_from_build"], 0)
        old["notes"] = "• Improve CI"
        old["changelog"][0]["notes"] = "• Improve CI"
        old["commit"] = newest
        rerun = self.produce(self.latest(1, newest), old)
        self.assertEqual(rerun["notes"], "• No user-facing changes in this build.")
        self.assertEqual(rerun["changelog"][0]["notes"], rerun["notes"])

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
