#!/usr/bin/env python3
"""The version of a native release: its New York release date plus a daily revision.

    2026.11.19.3   version, shown by every client, the update feeds and Google Play
    2026.11.1903   app version (YEAR.MONTH.DAY*100+N) where a platform takes three
                   integers: CFBundleShortVersionString
    2026111903     build (YYYYMMDDNN), the number clients compare: CFBundleVersion,
                   Android versionCode and the self-updater's build

Each release.yml run is one release, dated by when the run was created. Its
revision is one more than the earlier release runs created that New York day
whose first attempt started; runs cancelled while they waited never built
anything. GitHub never changes what a first attempt did, so re-runs keep their
version and no earlier release is ever renumbered.

    native_release_version.py   # in release.yml; needs GH_TOKEN, GITHUB_REPOSITORY, GITHUB_RUN_ID
"""

from datetime import datetime, time, timedelta, timezone
import json
import os
import subprocess
import sys
from zoneinfo import ZoneInfo

RELEASE_TIME_ZONE = ZoneInfo("America/New_York")
MAX_RELEASES_PER_DAY = 99


def release_date(timestamp):
    """The New York calendar date of an ISO-8601 UTC timestamp such as GitHub's created_at."""
    return datetime.fromisoformat(timestamp.replace("Z", "+00:00")).astimezone(RELEASE_TIME_ZONE).date()


def day_bounds(day):
    """The UTC range of one New York day, as GitHub's `created` filter takes it."""
    start, end = (
        datetime.combine(value, time(), RELEASE_TIME_ZONE).astimezone(timezone.utc)
        for value in (day, day + timedelta(days=1))
    )
    end -= timedelta(seconds=1)
    return f"{start:%Y-%m-%dT%H:%M:%SZ}..{end:%Y-%m-%dT%H:%M:%SZ}"


def release_version(day, revision):
    if not 1 <= revision <= MAX_RELEASES_PER_DAY:
        raise ValueError(f"Caper supports 1 to {MAX_RELEASES_PER_DAY} native releases a day; {day} needs {revision}")
    return {
        "version": f"{day:%Y.%m.%d}.{revision}",
        "app_version": f"{day.year}.{day.month}.{day.day * 100 + revision}",
        "build": f"{day:%Y%m%d}{revision:02d}",
    }


def started(run, first_attempt_jobs):
    """Whether a run's first attempt ran any job, so it may have built and uploaded apps."""
    if run["run_attempt"] == 1 and run["conclusion"] in ("success", "failure"):
        return True
    return any(job["conclusion"] != "skipped" for job in first_attempt_jobs(run["id"]))


def resolve(run, same_day_runs, first_attempt_jobs):
    """The version of `run`, given the release runs created on its New York day."""
    if run["run_attempt"] > 1 and not started({**run, "run_attempt": 1, "conclusion": None}, first_attempt_jobs):
        raise ValueError(
            "this run was cancelled before it started; start a new Native release instead of re-running it"
        )
    earlier = [other for other in same_day_runs if other["run_number"] < run["run_number"]]
    revision = 1 + sum(started(other, first_attempt_jobs) for other in earlier)
    return release_version(release_date(run["created_at"]), revision)


def gh(path):
    pages = subprocess.run(
        ["gh", "api", "--paginate", "--slurp", path], check=True, capture_output=True, text=True
    ).stdout
    return json.loads(pages)


def main():
    repo, run_id = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"]
    [run] = gh(f"repos/{repo}/actions/runs/{run_id}")
    created = day_bounds(release_date(run["created_at"]))
    runs = [
        other
        for page in gh(f"repos/{repo}/actions/workflows/{run['workflow_id']}/runs?created={created}&per_page=100")
        for other in page["workflow_runs"]
    ]

    def first_attempt_jobs(other_id):
        return [
            job
            for page in gh(f"repos/{repo}/actions/runs/{other_id}/attempts/1/jobs?per_page=100")
            for job in page["jobs"]
        ]

    try:
        version = resolve(run, runs, first_attempt_jobs)
    except ValueError as error:
        sys.exit(str(error))
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as file:
            file.writelines(f"{key}={value}\n" for key, value in version.items())
    print(json.dumps(version))


if __name__ == "__main__":
    main()
