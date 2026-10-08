#!/usr/bin/env python3
"""Upload a signed Android App Bundle to a Google Play track and roll it out.

Uses the Google Play Developer API with a service account that has release
permissions for the app in Play Console. The track defaults to "internal",
whose testers get the update through the Play Store within minutes.

Environment: GOOGLE_PLAY_SERVICE_ACCOUNT (path to the JSON key),
PLAY_PACKAGE (default chat.caper.android), PLAY_TRACK (default internal),
PLAY_RELEASE_NAME, PLAY_RELEASE_NOTES (optional). Argument: the .aab path.
Requires PyJWT with the cryptography backend.
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import jwt

API = "https://androidpublisher.googleapis.com/androidpublisher/v3/applications"
UPLOAD = "https://androidpublisher.googleapis.com/upload/androidpublisher/v3/applications"
SCOPE = "https://www.googleapis.com/auth/androidpublisher"


def access_token(account: dict) -> str:
    now = int(time.time())
    assertion = jwt.encode(
        {"iss": account["client_email"], "scope": SCOPE, "aud": account["token_uri"], "iat": now, "exp": now + 3600},
        account["private_key"],
        algorithm="RS256",
        headers={"kid": account.get("private_key_id")},
    )
    form = urllib.parse.urlencode(
        {
            "grant_type": "urn:ietf:params:oauth:grant-type:jwt-bearer",
            "assertion": assertion,
        }
    ).encode()
    with urllib.request.urlopen(urllib.request.Request(account["token_uri"], data=form), timeout=60) as response:
        return json.load(response)["access_token"]


def call(
    token: str, method: str, url: str, body: dict | bytes | None = None, content_type: str = "application/json"
) -> tuple[int, dict]:
    data = body if isinstance(body, bytes) else (json.dumps(body).encode() if body is not None else None)
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": content_type,
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=600) as response:
            raw = response.read()
            return response.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            return error.code, json.loads(raw)
        except ValueError:
            return error.code, {"raw": raw.decode(errors="replace")}


def must(result: tuple[int, dict], what: str) -> dict:
    status, body = result
    if status >= 300:
        sys.exit(f"{what} failed with HTTP {status}: {json.dumps(body)[:2000]}")
    return body


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: play-upload.py BUNDLE.aab")
    bundle = sys.argv[1]
    with open(os.environ["GOOGLE_PLAY_SERVICE_ACCOUNT"], encoding="utf-8") as key:
        account = json.load(key)
    package = os.environ.get("PLAY_PACKAGE", "chat.caper.android")
    track = os.environ.get("PLAY_TRACK", "internal")
    name = os.environ.get("PLAY_RELEASE_NAME", "")
    notes = os.environ.get("PLAY_RELEASE_NOTES", "").strip()[:500]

    token = access_token(account)
    edit = must(call(token, "POST", f"{API}/{package}/edits", {}), "Starting a Play edit")["id"]
    with open(bundle, "rb") as file:
        uploaded = must(
            call(
                token,
                "POST",
                f"{UPLOAD}/{package}/edits/{edit}/bundles?uploadType=media",
                file.read(),
                "application/octet-stream",
            ),
            "Uploading the bundle",
        )
    version_code = str(uploaded["versionCode"])
    print(f"Uploaded {bundle} as version code {version_code}")

    def release(status: str) -> dict:
        entry: dict = {"versionCodes": [version_code], "status": status}
        if name:
            entry["name"] = name
        if notes:
            entry["releaseNotes"] = [{"language": "en-US", "text": notes}]
        return {"track": track, "releases": [entry]}

    rollout = "completed"
    status, body = call(token, "PUT", f"{API}/{package}/edits/{edit}/tracks/{track}", release(rollout))
    if status >= 300 and "draft" in json.dumps(body).lower():
        rollout = "draft"
        # Play only accepts draft releases until the app has been published once.
        print("::warning::Play accepted this only as a draft release; roll it out in Play Console.")
        status, body = call(token, "PUT", f"{API}/{package}/edits/{edit}/tracks/{track}", release(rollout))
    must((status, body), f"Assigning the {track} track")
    must(call(token, "POST", f"{API}/{package}/edits/{edit}:commit", {}), "Committing the Play edit")
    if rollout == "completed":
        print(f"Version code {version_code} is rolling out to {track} testers")
    else:
        print(f"Version code {version_code} is saved as a draft on the {track} track")


if __name__ == "__main__":
    main()
