from datetime import date
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "native_release_version", Path(__file__).resolve().parents[1] / "scripts/native_release_version.py"
)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def run(number, created_at="2026-11-19T15:00:00Z", conclusion="success", attempt=1):
    return {
        "id": number * 10,
        "run_number": number,
        "created_at": created_at,
        "conclusion": conclusion,
        "run_attempt": attempt,
    }


class ReleaseVersionTests(unittest.TestCase):
    def test_dates_releases_in_new_york_across_utc_midnight_and_daylight_saving(self):
        self.assertEqual(release.release_date("2026-11-20T04:30:00Z"), date(2026, 11, 19))
        self.assertEqual(release.release_date("2026-11-20T05:30:00Z"), date(2026, 11, 20))
        self.assertEqual(release.release_date("2026-07-20T03:30:00Z"), date(2026, 7, 19))
        # The day clocks fall back lasts 25 hours.
        self.assertEqual(release.day_bounds(date(2026, 11, 1)), "2026-11-01T04:00:00Z..2026-11-02T04:59:59Z")

    def test_formats_each_version_and_builds_always_increase(self):
        self.assertEqual(
            release.release_version(date(2026, 1, 5), 3),
            {"version": "2026.01.05.3", "app_version": "2026.1.503", "build": "2026010503"},
        )
        self.assertEqual(
            release.release_version(date(2026, 11, 19), 12),
            {"version": "2026.11.19.12", "app_version": "2026.11.1912", "build": "2026111912"},
        )
        builds = [
            int(release.release_version(day, revision)["build"])
            for day, revision in [(date(2026, 11, 19), 9), (date(2026, 11, 19), 10), (date(2026, 11, 20), 1)]
        ]
        self.assertEqual(builds, sorted(builds))
        # Above every 0.1.<run> build, and below Android's versionCode limit.
        self.assertTrue(10_000 < builds[0] and builds[-1] < 2_100_000_000)
        for revision in (0, 100):
            with self.assertRaises(ValueError):
                release.release_version(date(2026, 11, 19), revision)

    def test_counts_only_earlier_runs_that_started(self):
        jobs = {20: [], 30: [{"conclusion": "cancelled"}], 40: [{"conclusion": "skipped"}]}
        asked = []

        def first_attempt_jobs(run_id):
            asked.append(run_id)
            return jobs[run_id]

        today = [
            run(1),  # Released.
            run(2, conclusion="cancelled"),  # Cancelled while waiting: never built.
            run(3, conclusion="cancelled"),  # Cancelled mid-build: may have uploaded.
            run(4, conclusion="skipped"),
            run(5, conclusion="failure"),
            run(6, conclusion=None),
            run(7, conclusion=None),
        ]
        version = release.resolve(today[5], today, first_attempt_jobs)
        self.assertEqual(version["version"], "2026.11.19.4")
        self.assertEqual(sorted(asked), [20, 30, 40])

    def test_reruns_keep_their_version_and_runs_that_never_started_cannot_rerun(self):
        jobs = {20: [], 30: [{"conclusion": "success"}]}
        today = [run(1), run(2, conclusion="cancelled"), run(3, conclusion=None)]
        first = release.resolve(today[2], today, jobs.get)
        # Run 2 was re-run later; what its first attempt did has not changed.
        today[1] = run(2, conclusion="success", attempt=2)
        today[2] = run(3, conclusion=None, attempt=2)
        self.assertEqual(release.resolve(today[2], today, jobs.get), first)
        with self.assertRaisesRegex(ValueError, "cancelled before it started"):
            release.resolve(today[1], today, jobs.get)


if __name__ == "__main__":
    unittest.main()
