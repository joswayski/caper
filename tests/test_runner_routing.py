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
                    {"windows": "false", "linux": "false", "linux_arm": "true"},
                )

    def test_linux_arm_routes_fail_closed(self):
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
                outputs = self.route(**overrides)
                self.assertEqual(outputs["linux_arm"], "false")

    def test_fallback_never_cancels_unfinished_local_mac_work(self):
        workflow = WORKFLOW.with_name("mac-runner-fallback.yml")
        script = textwrap.dedent(workflow.read_text().split("        run: |\n", 1)[1])
        windows = {"status": "queued", "labels": ["self-hosted", "josewin"], "created_at": "2000-01-01T00:00:00Z"}
        for label in ("josemac-ui", "josemac-sim"):
            for status in ("queued", "in_progress", "completed"):
                with self.subTest(label=label, status=status), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    gh = root / "gh"
                    gh.write_text(
                        "#!/usr/bin/env python3\n"
                        + textwrap.dedent("""\
                            import os
                            from pathlib import Path
                            import subprocess
                            import sys

                            path = next(arg for arg in sys.argv if arg.startswith("repos/"))
                            if "/jobs?" in path:
                                if "--jq" in sys.argv:
                                    subprocess.run(["jq", sys.argv[sys.argv.index("--jq") + 1]],
                                                   input=os.environ["TEST_JOBS"], text=True, check=True)
                                else:
                                    print(os.environ["TEST_JOBS"])
                            elif "/actions/runs?status=queued" in path:
                                print("123 456 main")
                            elif "/actions/runs?status=in_progress" in path:
                                pass
                            elif path.endswith(("/cancel", "/rerun")):
                                with Path(os.environ["TEST_ACTIONS"]).open("a") as output:
                                    output.write(path.rsplit("/", 1)[-1] + "\\n")
                            elif path.endswith("/runs/123"):
                                print("completed")
                            elif "/actions/workflows/456/runs?" in path:
                                pass
                            else:
                                raise SystemExit("Unexpected API call: " + path)
                            """)
                    )
                    gh.chmod(0o755)
                    actions = root / "actions"
                    env = {
                        **os.environ,
                        "PATH": f"{root}:{os.environ['PATH']}",
                        "REPO": "fixture/repository",
                        "RUNNER_TOKEN": "",
                        "MAX_WAIT_MINUTES": "60",
                        "MAX_BUSY_WAIT_MINUTES": "180",
                        "TEST_ACTIONS": str(actions),
                        "TEST_JOBS": json.dumps(
                            {"jobs": [windows, {**windows, "labels": ["self-hosted", label], "status": status}]}
                        ),
                    }
                    result = subprocess.run(["bash", "-e", "-c", script], env=env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(
                        actions.read_text() if actions.exists() else "",
                        "cancel\nrerun\n" if status == "completed" else "",
                    )


if __name__ == "__main__":
    unittest.main()
