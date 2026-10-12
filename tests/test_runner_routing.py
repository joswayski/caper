import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest


WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/mac-route.yml"


class RunnerRoutingTests(unittest.TestCase):
    def route(self, **overrides):
        # Execute the workflow's shell entry point, with only the GitHub API
        # replaced. Assertions cover the outputs consumed by calling workflows.
        script = textwrap.dedent(WORKFLOW.read_text().split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            curl = root / "curl"
            curl.write_text('#!/usr/bin/env bash\nprintf "%s" "$RUNNER_RESPONSE"\n')
            curl.chmod(0o755)
            output = root / "outputs"
            env = {
                **os.environ,
                "PATH": f"{root}:{os.environ['PATH']}",
                "MAC": "on",
                "WINDOWS": "on",
                "LINUX": "on",
                "EVENT": "push",
                "REF": "refs/heads/main",
                "ATTEMPT": "1",
                "REPO": "fixture/repository",
                "TOKEN": "fixture-token",
                "GITHUB_OUTPUT": str(output),
                "GITHUB_STEP_SUMMARY": str(root / "summary"),
                "RUNNER_RESPONSE": json.dumps(
                    {
                        "runners": [
                            {"status": "online", "labels": [{"name": label}]}
                            for label in ("josemac", "josewin", "joselinux", "joselinux-arm")
                        ]
                    }
                ),
                **overrides,
            }
            result = subprocess.run(["bash", "-e", "-c", script], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            return dict(line.split("=", 1) for line in output.read_text().splitlines())

    def test_quarantined_desktops_stay_hosted_even_when_online_and_enabled(self):
        for event in ("push", "workflow_dispatch"):
            with self.subTest(event=event):
                self.assertEqual(
                    self.route(EVENT=event),
                    {"mac": "false", "windows": "false", "linux": "false", "linux_arm": "true"},
                )

    def test_arm_route_still_fails_closed(self):
        for overrides in (
            {"MAC": "off"},
            {"EVENT": "pull_request"},
            {"REF": "refs/heads/feature"},
            {"ATTEMPT": "2"},
            {"TOKEN": ""},
            {"RUNNER_RESPONSE": "unavailable"},
            {"RUNNER_RESPONSE": '{"runners": []}'},
        ):
            with self.subTest(overrides=overrides):
                self.assertEqual(self.route(**overrides)["linux_arm"], "false")


if __name__ == "__main__":
    unittest.main()
