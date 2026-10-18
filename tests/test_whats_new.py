import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import urllib.error


spec = importlib.util.spec_from_file_location(
    "scripts.whats_new", Path(__file__).resolve().parents[1] / "scripts/whats_new.py"
)
notes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notes)

PLATFORMS = ("web", "android", "ios", "macos", "windows", "linux")
DATE = "2026-10-11T19:00:00Z"


def encoded(value):
    return (json.dumps(value, ensure_ascii=False) + "\n").encode()


def snapshot(platform, revision, sequence=1, history=None, highlights=None):
    release = {"sequence": sequence, "revision": revision, "publishedAt": DATE, "highlights": highlights or []}
    return {
        "schema": 1,
        "platform": platform,
        "revision": revision,
        "sequence": sequence,
        "releases": [*(history or []), release],
    }


def highlight(
    commits, platforms=("web",), title="Easier channel navigation", body="Move between channels with clearer controls."
):
    return {"title": title, "body": body, "platforms": list(platforms), "commits": list(commits)}


class FakeGitHub:
    """Disposable remote storage. No GH_TOKEN or network is used by this boundary."""

    def __init__(self):
        self.feeds = {}
        self.created = {}
        self.events = []
        self.clock = 0
        self.fail = None
        self.before_upload = None

    def put(self, platform, name, raw):
        self.clock += 1
        self.feeds.setdefault(platform, {})[name] = raw
        self.created.setdefault(platform, {})[name] = f"2026-10-11T19:{self.clock // 60:02}:{self.clock % 60:02}Z"

    def seed(self, value):
        platform = value["platform"]
        self.put(platform, f"{value['revision']}.json", encoded(value))
        self.put(platform, "latest.json", encoded(value))

    def assets(self, platform):
        return dict(self.created[platform]) if platform in self.feeds else None

    def download(self, platform, name):
        return self.feeds[platform][name]

    def create(self, platform, revision):
        self.events.append((platform, "create"))
        self.feeds[platform] = {}
        self.created[platform] = {}

    def upload(self, platform, path, *, latest=False):
        if self.before_upload:
            self.before_upload()
        if self.fail == (platform, path.name):
            raise notes.GenerationError("fake publication interruption")
        if not latest and path.name in self.feeds[platform]:
            raise AssertionError("attempted to replace immutable history")
        self.events.append((platform, path.name))
        self.put(platform, path.name, path.read_bytes())


class HighlightsTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.repo = Path(self.directory.name)
        self.output = self.repo / "output"
        self.github = FakeGitHub()
        self.git("init", "-q")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.test")
        self.base = self.commit("Already shipped", {"apps/web/src/chat.ts": "old\n"})
        self.environment = patch.dict(
            os.environ,
            {
                "ANTHROPIC_API_KEY": "fake-api-key",
                "ANTHROPIC_MODEL": "",
                "ANTHROPIC_EFFORT": "",
                "ANTHROPIC_WORKSPACE_ID": "",
            },
        )
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.requests = []
        self.response = {"highlights": []}
        self.message = None
        self.provider_error = None
        self.http = patch.object(notes.urllib.request, "urlopen", side_effect=self.provider)
        self.http.start()
        self.addCleanup(self.http.stop)

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "commit.gpgsign=false", "-C", str(self.repo), *args],
            text=True,
        ).strip()

    def commit(self, subject, files=None, body=None, author=None):
        files = {"apps/web/src/chat.ts": subject + "\n"} if files is None else files
        for name, contents in files.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            if contents is None:
                path.unlink()
            else:
                path.write_text(contents)
        # Output files are not source evidence in the disposable repository.
        self.git(
            "add",
            "apps",
            *(["shared"] if (self.repo / "shared").exists() else []),
            *(["docs"] if (self.repo / "docs").exists() else []),
        )
        self.git(
            "commit",
            "--allow-empty",
            "-qm",
            subject,
            *(["-m", body] if body else []),
            *(["--author", author] if author else []),
        )
        return self.git("rev-parse", "HEAD")

    def provider(self, request, timeout):
        body = json.loads(request.data)
        self.requests.append(body)
        self.assertEqual(request.full_url, "https://api.anthropic.com/v1/messages")
        self.assertEqual(request.get_header("X-api-key"), "fake-api-key")
        self.assertEqual(request.get_header("Anthropic-beta"), "server-side-fallback-2026-07-01")
        self.assertEqual(request.get_header("Anthropic-workspace-id"), os.environ["ANTHROPIC_WORKSPACE_ID"] or None)
        self.assertEqual(timeout, 300)
        self.assertEqual(body["max_tokens"], 16000)
        self.assertEqual(body["fallbacks"], "default")
        self.assertNotIn("thinking", body)
        self.assertEqual(body["output_config"]["format"]["type"], "json_schema")
        schema = json.dumps(body["output_config"]["format"]["schema"])
        for unsupported in ("minLength", "maxLength", "maxItems"):
            self.assertNotIn(f'"{unsupported}"', schema)
        self.assertLessEqual(len(request.data.decode()), notes.MAX_INPUT_CHARS)
        self.assertNotIn("tools", body)
        if self.provider_error:
            raise self.provider_error
        message = self.message or {
            "type": "message",
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": json.dumps(self.response)}],
        }
        return io.BytesIO(message if isinstance(message, bytes) else encoded(message))

    def generate(self, revision=None, audience="web", publish=True, native_version=None):
        return notes.generate(
            revision or self.git("rev-parse", "HEAD"),
            audience,
            self.output,
            repo=self.repo,
            github=self.github,
            publish=publish,
            native_version=native_version,
        )

    def seed(self, revision=None, platforms=("web",)):
        for platform in platforms:
            self.github.seed(snapshot(platform, revision or self.base))

    def test_first_baseline_is_silent_and_all_native_outputs_exist_before_publication(self):
        revision = self.commit("A feature already installed", {"shared/design.css": "new\n"})

        def prepared():
            for platform in PLATFORMS[1:]:
                self.assertTrue((self.output / platform / f"{revision}.json").is_file())
                self.assertTrue((self.output / platform / "latest.json").is_file())

        self.github.before_upload = prepared
        result = self.generate(revision, "native")
        self.assertEqual(self.requests, [])
        self.assertEqual(set(result), set(PLATFORMS[1:]))
        for platform, value in result.items():
            self.assertEqual(value["sequence"], 1)
            self.assertEqual(value["releases"][0]["highlights"], [])
            self.assertEqual(value["revision"], revision)
            self.assertEqual(value["platform"], platform)
            self.assertEqual(len(value["releases"]), 1)
        uploads = [name for _, name in self.github.events if name != "create"]
        self.assertEqual(uploads, [f"{revision}.json"] * 5 + ["latest.json"] * 5)

    def test_exact_multiple_commit_range_and_api_context(self):
        self.seed()
        first = self.commit("Improve channel selection", body="Keep the selected channel visible.")
        api = self.commit("Persist the selected channel", {"apps/api/src/channels.rs": "persist();\n"})
        newest = self.commit("Fix narrow channel controls", {"apps/web/src/mobile.ts": "narrow();\n"})
        future = self.commit("Unshipped feature", {"apps/web/src/future.ts": "future();\n"})
        self.response = {"highlights": [highlight([first, api, newest])]}
        result = self.generate(newest)["web"]
        evidence = json.loads(self.requests[0]["messages"][0]["content"])
        self.assertEqual(evidence["platformRanges"], {"web": [first, api, newest]})
        self.assertEqual([item["commit"] for item in evidence["commits"]], [first, api, newest])
        self.assertIn("Keep the selected channel visible.", evidence["commits"][0]["body"])
        self.assertIn("+narrow();", evidence["commits"][-1]["diff"])
        self.assertNotIn(future, self.requests[0]["messages"][0]["content"])
        self.assertEqual(len(self.requests), 1)
        self.assertEqual([entry["revision"] for entry in result["releases"]], [self.base, newest])
        self.assertEqual(result["sequence"], 2)
        self.assertEqual(
            result["releases"][-1]["highlights"],
            [{"title": "Easier channel navigation", "body": "Move between channels with clearer controls."}],
        )

    def test_first_parent_merge_does_not_cite_branch_implementation(self):
        self.seed()
        self.git("checkout", "-qb", "feature")
        implementation = self.commit("Hidden implementation")
        self.git("checkout", "-q", "-")
        self.git("merge", "--no-ff", "-qm", "Improve channel selection", "feature")
        merge = self.git("rev-parse", "HEAD")
        self.response = {"highlights": [highlight([merge])]}
        self.generate(merge)
        evidence = json.loads(self.requests[0]["messages"][0]["content"])
        self.assertEqual(evidence["platformRanges"]["web"], [merge])
        self.assertNotIn(implementation, self.requests[0]["messages"][0]["content"])
        self.assertIn("+Hidden implementation", evidence["commits"][0]["diff"])

    def test_maintenance_tests_secret_dependencies_and_server_only_are_quiet(self):
        self.seed()
        self.commit("docs: explain behavior", {"docs/behavior.md": "docs"})
        self.commit("refactor: channel controls")
        self.commit("Update runtime dependencies", {"apps/web/src/runtime.ts": "dep"})
        self.commit(
            "Improve regression checks",
            {
                "apps/web/src/chat.test.ts": "test",
                "apps/web/src/__fixtures__/chat.ts": "test",
                "shared/messages/link-cases.json": "test",
            },
        )
        self.commit(
            "Rotate credentials",
            {
                "apps/web/src/.env.local": "secret",
                "shared/credentials.json": "secret",
                "apps/web/src/cert.pem": "secret",
            },
        )
        self.commit("Refresh implementation", author="dependabot[bot] <bot@example.test>")
        newest = self.commit("Persist channel selection", {"apps/api/src/channels.rs": "persist();"})
        result = self.generate(newest)["web"]
        self.assertEqual(self.requests, [])
        self.assertEqual(result["sequence"], 2)
        self.assertEqual(result["releases"][-1]["highlights"], [])

    def test_grouped_platform_highlights_use_each_platform_baseline(self):
        self.seed(platforms=PLATFORMS[1:])
        first = self.commit("Improve mobile voice controls", {"apps/native/android/app/src/main/Voice.kt": "mobile();"})
        self.github.seed(snapshot("android", first, 2, snapshot("android", self.base)["releases"]))
        shared = self.commit("Improve shared voice indicators", {"shared/design.css": "voice {}"})
        ios = self.commit("Improve iPhone controls", {"apps/native/apple/Sources/CaperIOS/Controls.swift": "iphone()"})
        newest = self.commit("Improve desktop controls", {"apps/native/desktop/src/voice.rs": "desktop();"})
        self.response = {
            "highlights": [
                highlight([shared], PLATFORMS[1:]),
                highlight([ios], ["ios"], "iPhone controls", "Clearer controls on iPhone."),
                highlight([newest], ["windows", "linux"], "Desktop controls", "Clearer controls on desktop."),
            ]
        }
        result = self.generate(newest, "native")
        self.assertEqual(len(self.requests), 1)
        evidence = json.loads(self.requests[0]["messages"][0]["content"])
        self.assertEqual(evidence["platformRanges"]["android"], [shared, ios, newest])
        self.assertEqual(evidence["platformRanges"]["ios"], [first, shared, ios, newest])
        self.assertEqual(result["android"]["sequence"], 3)
        self.assertEqual(len(result["android"]["releases"][-1]["highlights"]), 1)
        self.assertEqual(len(result["ios"]["releases"][-1]["highlights"]), 2)
        self.assertEqual(len(result["macos"]["releases"][-1]["highlights"]), 1)
        self.assertEqual(len(result["windows"]["releases"][-1]["highlights"]), 2)

    def test_platform_evidence_cannot_cross_different_release_ranges(self):
        self.seed(platforms=PLATFORMS[1:])
        old = self.commit("Improve shared controls", {"shared/design.css": "controls {}"})
        self.github.seed(snapshot("android", old, 2, snapshot("android", self.base)["releases"]))
        newest = self.commit(
            "Improve iPhone controls", {"apps/native/apple/Sources/CaperIOS/Controls.swift": "iphone()"}
        )
        self.response = {"highlights": [highlight([old], ["android", "ios"])]}
        before = copy.deepcopy(self.github.feeds)
        with self.assertRaises(notes.GenerationError):
            self.generate(newest, "native")
        self.assertEqual(self.github.feeds, before)
        self.assertFalse(self.output.exists())

    def test_platform_specific_runtime_cannot_announce_other_clients(self):
        self.seed(platforms=PLATFORMS[1:])
        ios = self.commit("Improve iPhone controls", {"apps/native/apple/Sources/CaperIOS/Controls.swift": "iphone()"})
        mac = self.commit("Improve Mac updater", {"apps/native/apple/Sources/CaperMacOS/AppUpdater.swift": "update()"})
        updater = self.commit("Improve signed update feedback", {"apps/native/updater/src/main.rs": "check();"})
        newest = self.commit("Improve server persistence", {"apps/api/src/persist.rs": "persist();"})
        before = copy.deepcopy(self.github.feeds)
        for evidence, platform in (
            (ios, "macos"),
            (mac, "ios"),
            (updater, "ios"),
            (updater, "android"),
            (newest, "windows"),
        ):
            with self.subTest(platform=platform):
                self.response = {"highlights": [highlight([evidence], [platform])]}
                with self.assertRaises(notes.GenerationError):
                    self.generate(newest, "native")
                self.assertEqual(self.github.feeds, before)
                self.assertFalse(self.output.exists())
        self.response = {"highlights": [highlight([updater], ["macos", "windows", "linux"])]}
        result = self.generate(newest, "native")
        for platform in PLATFORMS[1:]:
            self.assertEqual(
                len(result[platform]["releases"][-1]["highlights"]), int(platform in {"macos", "windows", "linux"})
            )

    def test_invalid_schema_evidence_and_text_fail_before_any_publication(self):
        self.seed()
        api = self.commit("Improve server persistence", {"apps/api/src/persist.rs": "persist();"})
        newest = self.commit("Improve channel controls")
        good = highlight([newest])
        variants = [
            {**good, "commits": [self.base]},
            {**good, "commits": ["f" * 40]},
            {**good, "commits": [api]},
            {**good, "platforms": ["android"]},
            {**good, "platforms": ["web", "web"]},
            {**good, "commits": []},
            {**good, "revision": newest},
            {**good, "title": "x" * 101},
            {**good, "body": "x" * 501},
            {**good, "title": "<b>Controls</b>"},
            {**good, "body": "Controls\nHidden"},
            {**good, "body": "50% faster navigation"},
            {**good, "body": "Deployed to all users."},
            {**good, "title": ""},
        ]
        before = copy.deepcopy(self.github.feeds)
        for invalid in [
            *({"highlights": [item]} for item in variants),
            {"highlights": [good] * 6},
            {"highlights": [], "extra": True},
        ]:
            with self.subTest(invalid=invalid):
                self.response = invalid
                with self.assertRaises(notes.GenerationError):
                    self.generate(newest)
                self.assertEqual(self.github.feeds, before)
                self.assertEqual(self.github.events, [])
                self.assertFalse(self.output.exists())

    def test_refusal_incomplete_error_malformed_and_oversized_provider_responses_fail_closed(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        responses = [
            {"type": "message", "stop_reason": "refusal", "content": []},
            {"type": "message", "stop_reason": "max_tokens", "content": []},
            {"type": "error", "error": {"message": "secret error body"}},
            {"type": "message", "stop_reason": "end_turn", "content": [{"type": "refusal"}]},
            {"type": "message", "stop_reason": "end_turn", "content": [{"type": "thinking", "thinking": ""}]},
            {
                "type": "message",
                "stop_reason": "end_turn",
                "content": [{"type": "text", "text": '{"highlights":[]}'}, {"type": "text", "text": "extra"}],
            },
            {
                "type": "message",
                "stop_reason": "end_turn",
                "content": [{"type": "text", "text": '{"highlights":[],"highlights":[]}'}],
            },
            b"invalid secret response",
            b"x" * (128 * 1024 + 1),
        ]
        before = copy.deepcopy(self.github.feeds)
        for response in responses:
            with self.subTest(response=str(response)[:80]):
                self.message = response
                with self.assertRaises(notes.GenerationError):
                    self.generate(newest)
                self.assertEqual(self.github.feeds, before)
                self.assertFalse(self.output.exists())

    def test_http_timeout_and_error_do_not_log_raw_content(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        self.provider_error = urllib.error.HTTPError(
            "https://example.test", 500, "secret error body", {}, io.BytesIO(b"secret")
        )
        with patch.object(notes, "wait_before_retry") as sleep:
            self.assert_cli_failure(newest, "Claude request failed (HTTP 500 unknown_error)")
        self.assertEqual([call.args for call in sleep.call_args_list], [(10,), (30,)])
        self.assertEqual(len(self.requests), 3)
        self.requests.clear()
        self.provider_error = TimeoutError("secret timeout body")
        self.assert_cli_failure(newest, "Claude request failed")
        self.assertEqual(len(self.requests), 1)

    def test_transient_provider_errors_retry_before_publishing(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        self.response = {
            "highlights": [
                {
                    "title": "Clearer channel controls",
                    "body": "Channel controls are easier to find.",
                    "platforms": ["web"],
                    "commits": [newest],
                }
            ]
        }
        failures = [
            urllib.error.HTTPError("https://example.test", 529, "overloaded", {}, io.BytesIO(b"")),
            urllib.error.URLError(ConnectionResetError("reset")),
        ]
        provider = self.provider

        def flaky(request, timeout):
            if failures:
                self.requests.append(json.loads(request.data))
                raise failures.pop(0)
            return provider(request, timeout)

        with patch.object(notes.urllib.request, "urlopen", side_effect=flaky), patch.object(notes, "wait_before_retry"):
            feeds = self.generate(newest)
        self.assertEqual(len(self.requests), 3)
        self.assertEqual(feeds["web"]["releases"][-1]["highlights"][0]["title"], "Clearer channel controls")

    def test_workspace_id_is_sent_when_configured(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        with patch.dict(os.environ, {"ANTHROPIC_WORKSPACE_ID": "wrkspc_01Example"}):
            self.generate(newest)
        self.assertEqual(len(self.requests), 1)
        self.github.events.clear()
        with patch.dict(os.environ, {"ANTHROPIC_WORKSPACE_ID": "bad id\n"}):
            self.assert_cli_failure(self.commit("Improve channel menus"), "ANTHROPIC_WORKSPACE_ID")

    def test_drafted_answer_is_reused_only_for_identical_evidence(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        self.response = {
            "highlights": [
                {
                    "title": "Clearer channel controls",
                    "body": "Channel controls are easier to find.",
                    "platforms": ["web"],
                    "commits": [newest],
                }
            ]
        }
        answer = self.repo / "draft" / "answer.json"
        drafts = self.repo / "draft-output"
        notes.generate(newest, "web", drafts, repo=self.repo, github=self.github, publish=False, save_answer=answer)
        self.assertEqual(len(self.requests), 1)
        self.assertEqual(self.github.events, [])
        # Publishing with the draft asks nothing more and publishes its words.
        self.provider_error = TimeoutError("must not be asked")
        feeds = notes.generate(
            newest, "web", self.output, repo=self.repo, github=self.github, publish=True, answer=answer
        )
        self.assertEqual(len(self.requests), 1)
        self.assertEqual(feeds["web"]["releases"][-1]["highlights"][0]["title"], "Clearer channel controls")
        self.assertIn(("web", f"{newest}.json"), self.github.events)
        # A draft for different evidence, or a damaged one, is asked again.
        self.provider_error = None
        later = self.commit("Improve channel menus")
        self.response = {"highlights": []}
        drafted = answer.read_bytes()
        prompt = self.requests[0]["messages"][0]["content"]
        self.assertIsNotNone(notes.saved_answer(answer, newest, prompt))
        with patch.object(notes, "SYSTEM", notes.SYSTEM + "Changed rules."):
            self.assertIsNone(notes.saved_answer(answer, newest, prompt))
        for contents in (drafted, b"{not json"):
            answer.write_bytes(contents)
            before = len(self.requests)
            notes.generate(later, "web", drafts, repo=self.repo, github=self.github, publish=False, answer=answer)
            self.assertEqual(len(self.requests), before + 1)

    def test_permanent_provider_errors_do_not_retry(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        body = {"type": "error", "error": {"type": "invalid_request_error", "message": "max_tokens:\n too\x00 large"}}
        self.provider_error = urllib.error.HTTPError(
            "https://example.test", 400, "secret error body", {}, io.BytesIO(encoded(body))
        )
        with patch.object(notes, "wait_before_retry") as sleep:
            self.assert_cli_failure(
                newest, "Claude request failed (HTTP 400 invalid_request_error: max_tokens: too large)"
            )
        sleep.assert_not_called()
        self.assertEqual(len(self.requests), 1)

    def assert_cli_failure(self, revision, expected):
        before = {path.relative_to(self.output): path.read_bytes() for path in self.output.rglob("*.json")}
        stderr = io.StringIO()
        with (
            patch.object(notes, "GitHub", return_value=self.github),
            contextlib.chdir(self.repo),
            contextlib.redirect_stderr(stderr),
        ):
            status = notes.main(
                ["--revision", revision, "--audience", "web", "--output-dir", str(self.output), "--publish"]
            )
        self.assertEqual(status, 1)
        self.assertIn(expected, stderr.getvalue())
        self.assertNotIn("secret", stderr.getvalue())
        self.assertNotIn("Improve channel", stderr.getvalue())
        self.assertEqual(self.github.events, [])
        self.assertEqual(
            {path.relative_to(self.output): path.read_bytes() for path in self.output.rglob("*.json")}, before
        )

    def test_missing_api_key_fails_product_changes_but_not_baseline_or_silent_build(self):
        with patch.dict(os.environ, {}, clear=True):
            self.generate(self.base)
            silent = self.commit("chore: refresh controls")
            self.assertEqual(self.generate(silent)["web"]["releases"][-1]["highlights"], [])
            self.github.events.clear()
            newest = self.commit("Improve channel controls")
            self.assert_cli_failure(newest, "ANTHROPIC_API_KEY")
        self.assertEqual(self.requests, [])

    def test_defaults_to_opus_high_effort_and_variables_override(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        self.generate(newest)
        self.assertEqual(self.requests[0]["model"], "claude-opus-5-5")
        self.assertEqual(self.requests[0]["output_config"]["effort"], "high")
        later = self.commit("Improve channel search")
        with patch.dict(os.environ, {"ANTHROPIC_MODEL": "claude-sonnet-5-5", "ANTHROPIC_EFFORT": "xhigh"}):
            self.generate(later)
        self.assertEqual(self.requests[1]["model"], "claude-sonnet-5-5")
        self.assertEqual(self.requests[1]["output_config"]["effort"], "xhigh")

    def test_invalid_effort_fails_before_any_provider_call(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        with patch.dict(os.environ, {"ANTHROPIC_EFFORT": "extreme"}):
            self.assert_cli_failure(newest, "ANTHROPIC_EFFORT")
        self.assertEqual(self.requests, [])

    def test_thinking_and_fallback_blocks_surround_the_single_answer(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        self.message = {
            "type": "message",
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": "", "signature": "opaque"},
                {"type": "fallback", "from": {"model": "claude-opus-5-5"}, "to": {"model": "claude-opus-5"}},
                {"type": "text", "text": json.dumps({"highlights": [highlight([newest])]})},
            ],
        }
        result = self.generate(newest)["web"]
        self.assertEqual(
            result["releases"][-1]["highlights"],
            [{"title": "Easier channel navigation", "body": "Move between channels with clearer controls."}],
        )

    def test_zero_highlights_is_a_valid_silent_release(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        result = self.generate(newest)["web"]
        self.assertEqual(len(self.requests), 1)
        self.assertEqual(result["sequence"], 2)
        self.assertEqual(result["releases"][-1]["highlights"], [])

    def test_native_versions_are_recorded_without_relabeling_legacy_or_immutable_releases(self):
        self.seed(platforms=PLATFORMS[1:])
        newest = self.commit("Improve native notes", {"shared/design.css": "new controls\n"})
        self.response = {"highlights": [highlight([newest], PLATFORMS[1:])]}
        result = self.generate(newest, "native", native_version="0.1.112")
        for value in result.values():
            self.assertNotIn("version", value["releases"][0])
            self.assertEqual(value["releases"][1]["version"], "0.1.112")
        self.github.events.clear()
        self.assertEqual(self.generate(newest, "native", native_version="0.1.113"), result)
        self.assertEqual(self.github.events, [])
        later = self.commit("Another native update", {"shared/design.css": "next controls\n"})
        self.response = {"highlights": [highlight([later], PLATFORMS[1:])]}
        for value in self.generate(later, "native", native_version="0.1.114").values():
            self.assertEqual([release.get("version") for release in value["releases"]], [None, "0.1.112", "0.1.114"])
        with self.assertRaises(notes.GenerationError):
            self.generate(later, "web", native_version="0.1.114")
        for version in ["", "Release 2", "0.1.114\n", "0.1." + "1" * 64]:
            with self.assertRaises(notes.GenerationError):
                self.generate(later, "native", native_version=version)

    def test_exact_rerun_and_rollback_reuse_bytes_without_model_or_lowering_latest(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        self.response = {"highlights": [highlight([newest])]}
        result = self.generate(newest)["web"]
        immutable = self.github.feeds["web"][f"{newest}.json"]
        head = self.github.feeds["web"]["latest.json"]
        self.requests.clear()
        self.github.events.clear()
        self.assertEqual(self.generate(newest)["web"], result)
        self.assertEqual(self.generate(self.base)["web"]["sequence"], 1)
        self.assertEqual(self.github.feeds["web"]["latest.json"], head)
        self.assertEqual(self.github.feeds["web"][f"{newest}.json"], immutable)
        self.assertEqual(self.requests, [])
        self.assertEqual(self.github.events, [])
        self.assertEqual((self.output / "web" / "latest.json").read_bytes(), head)

    def test_unrecorded_historical_rebuild_is_quiet_and_advances_sequence(self):
        self.git("checkout", "-qb", "old-source")
        historical = self.commit("Historical controls")
        self.git("checkout", "-q", "-")
        newest = self.commit("Modern controls")
        self.seed(newest)
        result = self.generate(historical)["web"]
        self.assertEqual(self.requests, [])
        self.assertEqual(result["sequence"], 2)
        self.assertEqual(result["releases"][-1]["highlights"], [])

    def test_interrupted_latest_update_recovers_without_model_including_missing_latest(self):
        self.seed()
        newest = self.commit("Improve channel controls")
        self.response = {"highlights": [highlight([newest])]}
        self.github.fail = ("web", "latest.json")
        with self.assertRaises(notes.GenerationError):
            self.generate(newest)
        exact = self.github.feeds["web"][f"{newest}.json"]
        self.github.fail = None
        self.requests.clear()
        self.generate(newest)
        self.assertEqual(self.github.feeds["web"]["latest.json"], exact)
        del self.github.feeds["web"]["latest.json"]
        del self.github.created["web"]["latest.json"]
        self.generate(self.base)
        self.assertEqual(self.github.feeds["web"]["latest.json"], exact)
        self.assertEqual(self.requests, [])

    def test_new_revision_after_interruption_does_not_reuse_reserved_sequence(self):
        self.seed()
        interrupted = self.commit("Improve channel controls")
        self.response = {"highlights": [highlight([interrupted])]}
        self.github.fail = ("web", "latest.json")
        with self.assertRaises(notes.GenerationError):
            self.generate(interrupted)
        newest = self.commit("Improve voice controls")
        self.github.fail = None
        self.requests.clear()
        self.response = {"highlights": [highlight([newest])]}
        result = self.generate(newest)["web"]
        self.assertEqual(result["sequence"], 3)
        self.assertEqual([entry["revision"] for entry in result["releases"]], [self.base, interrupted, newest])
        evidence = json.loads(self.requests[0]["messages"][0]["content"])
        self.assertEqual(evidence["platformRanges"], {"web": [newest]})

    def test_native_partial_upload_retry_uses_one_request_for_only_missing_assets(self):
        self.seed(platforms=PLATFORMS[1:])
        newest = self.commit("Improve shared controls", {"shared/design.css": "controls {}"})
        self.response = {"highlights": [highlight([newest], PLATFORMS[1:])]}
        self.github.fail = ("macos", f"{newest}.json")
        with self.assertRaises(notes.GenerationError):
            self.generate(newest, "native")
        self.assertTrue(
            all(json.loads(self.github.feeds[platform]["latest.json"])["sequence"] == 1 for platform in PLATFORMS[1:])
        )
        android = self.github.feeds["android"][f"{newest}.json"]
        self.requests.clear()
        self.github.fail = None
        self.response = {"highlights": [highlight([newest], ["macos", "windows", "linux"])]}
        result = self.generate(newest, "native")
        self.assertEqual(len(self.requests), 1)
        self.assertEqual(self.github.feeds["android"][f"{newest}.json"], android)
        self.assertTrue(all(value["sequence"] == 2 for value in result.values()))
        evidence = json.loads(self.requests[0]["messages"][0]["content"])
        self.assertEqual(set(evidence["platformRanges"]), {"macos", "windows", "linux"})

    def test_over_budget_diffs_are_shortened_so_long_gaps_still_publish(self):
        self.seed()
        chat = self.commit("Improve channel controls", {"apps/web/src/chat.ts": "x\n" * 150_000})
        self.commit("Persist voice state", {"apps/api/src/voice.rs": "y\n" * 150_000})
        newest = self.commit("Improve voice controls", {"apps/web/src/voice.ts": "z\n" * 150_000})
        self.response = {"highlights": [highlight([chat, newest])]}
        result = self.generate(newest)["web"]
        self.assertEqual(len(self.requests), 1)
        evidence = json.loads(self.requests[0]["messages"][0]["content"])
        commits = {item["subject"]: item for item in evidence["commits"]}
        self.assertEqual(set(commits), {"Improve channel controls", "Persist voice state", "Improve voice controls"})
        # Server-only context loses its diff first; client diffs share the rest evenly.
        self.assertEqual(commits["Persist voice state"]["diff"], "")
        self.assertTrue(all(item["diffTruncated"] for item in commits.values()))
        client = [commits["Improve channel controls"]["diff"], commits["Improve voice controls"]["diff"]]
        self.assertEqual(len(client[0]), len(client[1]))
        self.assertGreater(len(client[0]), 100_000)
        self.assertEqual(len(result["releases"][-1]["highlights"]), 1)

    def test_small_releases_send_whole_diffs(self):
        self.seed()
        newest = self.commit("Improve channel controls", {"apps/web/src/chat.ts": "small();\n"})
        self.generate(newest)
        evidence = json.loads(self.requests[0]["messages"][0]["content"])
        self.assertNotIn("diffTruncated", evidence["commits"][0])
        self.assertIn("+small();", evidence["commits"][0]["diff"])

    def test_commit_messages_over_budget_fail_without_provider_or_publish(self):
        self.seed()
        (self.repo / "apps/web/src/chat.ts").write_text("changed\n")
        message = self.repo / "message.txt"  # not staged; only apps/ is added
        message.write_text("Improve channel controls\n\n" + "b" * (notes.MAX_INPUT_CHARS + 1))
        self.git("add", "apps")
        self.git("commit", "-qF", str(message))
        newest = self.git("rev-parse", "HEAD")
        with self.assertRaisesRegex(notes.GenerationError, "even without diffs"):
            self.generate(newest)
        self.assertEqual(self.requests, [])
        self.assertEqual(self.github.events, [])
        self.assertFalse(self.output.exists())

    def test_retained_history_stays_oldest_first_max30_and_below64k(self):
        history = [
            {
                "sequence": number,
                "revision": f"{number:040x}",
                "publishedAt": DATE,
                "highlights": [{"title": "界" * 100, "body": "界" * 500}] * 5,
            }
            for number in range(1, 7)
        ]
        # Each whole release is retained or dropped, never cut through its text.
        prior = snapshot("web", self.base, 7, history, [{"title": "界" * 100, "body": "界" * 500}] * 5)
        while len(encoded(prior)) > 64 * 1024:
            prior["releases"].pop(0)
        self.github.seed(prior)
        newest = self.commit("Improve controls")
        self.response = {"highlights": [highlight([newest], title="界" * 100, body="界" * 500)] * 5}
        result = self.generate(newest)["web"]
        self.assertLessEqual(len((self.output / "web" / f"{newest}.json").read_bytes()), 64 * 1024)
        self.assertLess(len(result["releases"]), len(prior["releases"]) + 1)
        self.assertEqual(result["releases"][-2], prior["releases"][-1])
        self.assertEqual(result["sequence"], 8)
        compact = snapshot(
            "web",
            self.base,
            30,
            [
                {"sequence": number, "revision": f"{number:040x}", "publishedAt": DATE, "highlights": []}
                for number in range(1, 30)
            ],
        )
        self.github = FakeGitHub()
        self.github.seed(compact)
        self.response = {"highlights": []}
        result = self.generate(newest)["web"]
        self.assertEqual([entry["sequence"] for entry in result["releases"]], list(range(2, 32)))

    def test_invalid_remote_snapshot_preserves_other_platforms_and_fails_closed(self):
        self.seed(platforms=PLATFORMS[1:])
        newest = self.commit("Improve shared controls", {"shared/design.css": "controls {}"})
        for sequence in (True, 9_007_199_254_740_992):
            self.github.put("linux", "latest.json", encoded(snapshot("linux", self.base, sequence)))
            before = copy.deepcopy(self.github.feeds)
            with self.assertRaises(notes.GenerationError):
                self.generate(newest, "native")
            self.assertEqual(self.github.feeds, before)
        self.assertEqual(self.requests, [])
        self.assertFalse(self.output.exists())

    def test_standalone_cli_baseline_with_fake_gh_and_invalid_revision(self):
        executable = self.repo / "bin" / "gh"
        executable.parent.mkdir()
        executable.write_text("""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import sys

args = sys.argv[1:]
root = Path(os.environ["FAKE_GITHUB"])
if args[0] == "api":
    directory = root / args[-1].split("/")[-1]
    if not directory.exists():
        print('HTTP/2.0 404 Not Found\\n\\n{"message":"not found"}')
        sys.exit(1)
    assets = [{"name": path.name, "created_at": "2026-10-11T19:00:00Z"} for path in directory.iterdir()]
    print("HTTP/2.0 200 OK\\n\\n" + json.dumps({"draft": False, "prerelease": True, "assets": assets}))
else:
    directory = root / args[2]
    if args[1] == "create":
        if "--prerelease" not in args:
            sys.exit(2)
        directory.mkdir(parents=True)
    elif args[1] == "download":
        name = args[args.index("--pattern") + 1]
        shutil.copyfile(directory / name, Path(args[args.index("--dir") + 1]) / name)
    elif args[1] == "upload":
        source = Path(args[3])
        target = directory / source.name
        if target.exists() and "--clobber" not in args:
            sys.exit(2)
        shutil.copyfile(source, target)
    else:
        sys.exit(2)
""")
        executable.chmod(0o755)
        environment = {
            **os.environ,
            "PATH": f"{executable.parent}:{os.environ['PATH']}",
            "GH_TOKEN": "fake-gh-token",
            "GITHUB_REPOSITORY": "joswayski/caper",
            "FAKE_GITHUB": str(self.repo / "fake-github"),
        }
        environment.pop("ANTHROPIC_API_KEY", None)
        args = [
            sys.executable,
            spec.origin,
            "--revision",
            self.base,
            "--audience",
            "web",
            "--output-dir",
            str(self.output),
        ]
        result = subprocess.run(args, cwd=self.repo, env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("prepared 1 platform", result.stdout)
        value = json.loads((self.output / "web" / f"{self.base}.json").read_bytes())
        self.assertEqual(value["sequence"], 1)
        self.assertEqual(value["releases"][-1]["highlights"], [])
        # Exercise the real gh adapter against a disposable fake executable,
        # including creation, upload, metadata parsing, download and exact reuse.
        args.append("--publish")
        result = subprocess.run(args, cwd=self.repo, env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        feed = self.repo / "fake-github" / "whats-new-web"
        immutable = (feed / f"{self.base}.json").read_bytes()
        self.assertEqual((feed / "latest.json").read_bytes(), immutable)
        result = subprocess.run(args, cwd=self.repo, env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.output / "web" / f"{self.base}.json").read_bytes(), immutable)
        args[3] = "MAIN"
        result = subprocess.run(args, cwd=self.repo, env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("40 lowercase hex", result.stderr)
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
