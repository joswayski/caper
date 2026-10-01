#!/usr/bin/env python3
"""Hand a freshly uploaded iOS build to every TestFlight tester group.

Uploading only makes a build appear in App Store Connect. This waits for Apple
to finish processing it, sets its "What to Test" text, adds it to each beta
group that does not already receive every build automatically, and submits it
for beta app review when an external group (such as the public link) needs it.

Environment: NOTARY_KEY_PATH (.p8), NOTARY_KEY_ID, NOTARY_ISSUER, BUILD_NUMBER,
CAPER_IOS_BUNDLE_ID (default chat.caper.ios), WHAT_TO_TEST (optional).
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

API = "https://api.appstoreconnect.apple.com/v1"
PROCESSING_TIMEOUT_S = 40 * 60


def token() -> str:
    with open(os.environ["NOTARY_KEY_PATH"], encoding="utf-8") as key:
        private_key = key.read()
    now = int(time.time())
    return jwt.encode(
        {"iss": os.environ["NOTARY_ISSUER"], "iat": now, "exp": now + 15 * 60, "aud": "appstoreconnect-v1"},
        private_key,
        algorithm="ES256",
        headers={"kid": os.environ["NOTARY_KEY_ID"], "typ": "JWT"},
    )


def call(method: str, path: str, body: dict | None = None, query: dict | None = None) -> tuple[int, dict]:
    url = f"{API}{path}"
    if query:
        url += "?" + urllib.parse.urlencode(query)
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(url, data=data, method=method, headers={
        "Authorization": f"Bearer {token()}",
        "Content-Type": "application/json",
    })
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            raw = response.read()
            return response.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            detail = json.loads(raw)
        except ValueError:
            detail = {"raw": raw.decode(errors="replace")}
        return error.code, detail


def must(result: tuple[int, dict], what: str) -> dict:
    status, body = result
    if status >= 300:
        sys.exit(f"{what} failed with HTTP {status}: {json.dumps(body)[:2000]}")
    return body


def main() -> None:
    bundle_id = os.environ.get("CAPER_IOS_BUNDLE_ID", "chat.caper.ios")
    build_number = os.environ["BUILD_NUMBER"]
    notes = os.environ.get("WHAT_TO_TEST", "").strip()[:4000]

    apps = must(call("GET", "/apps", query={"filter[bundleId]": bundle_id}), "Looking up the app")["data"]
    if not apps:
        sys.exit(f"No App Store Connect app has bundle ID {bundle_id}.")
    app_id = apps[0]["id"]

    # Apple lists the build a few minutes after upload and then processes it.
    deadline = time.monotonic() + PROCESSING_TIMEOUT_S
    build = None
    while time.monotonic() < deadline:
        found = must(call("GET", "/builds", query={
            "filter[app]": app_id, "filter[version]": build_number, "limit": "1",
        }), "Looking up the build")["data"]
        state = found[0]["attributes"].get("processingState") if found else "NOT_LISTED"
        print(f"Build {build_number}: {state}", flush=True)
        if state == "VALID":
            build = found[0]
            break
        if state in {"FAILED", "INVALID"}:
            sys.exit(f"App Store Connect rejected build {build_number} ({state}); see the email from Apple.")
        time.sleep(30)
    if build is None:
        sys.exit(f"Build {build_number} was not processed within {PROCESSING_TIMEOUT_S // 60} minutes.")
    build_id = build["id"]

    if notes:
        localizations = must(call("GET", f"/builds/{build_id}/betaBuildLocalizations"), "Reading What to Test")["data"]
        english = next((item for item in localizations if item["attributes"].get("locale") == "en-US"), None)
        if english:
            must(call("PATCH", f"/betaBuildLocalizations/{english['id']}", {"data": {
                "type": "betaBuildLocalizations", "id": english["id"], "attributes": {"whatsNew": notes},
            }}), "Updating What to Test")
        else:
            must(call("POST", "/betaBuildLocalizations", {"data": {
                "type": "betaBuildLocalizations",
                "attributes": {"locale": "en-US", "whatsNew": notes},
                "relationships": {"build": {"data": {"type": "builds", "id": build_id}}},
            }}), "Setting What to Test")

    groups = must(call("GET", f"/apps/{app_id}/betaGroups", query={"limit": "200"}), "Listing tester groups")["data"]
    if not groups:
        # Without a group the build sits in App Store Connect and no phone,
        # including the developer's, ever receives it. Fail so the release
        # (and its Discord message) does not report the iPhone as shipped.
        sys.exit(
            f"Build {build_number} is in App Store Connect, but no TestFlight tester group exists, "
            "so no tester will get it. In App Store Connect → Caper Chat → TestFlight, create an "
            "Internal Testing group with Automatic Distribution on and add yourself, then add this build to it."
        )
    external = False
    for group in groups:
        attributes = group["attributes"]
        name = attributes.get("name", group["id"])
        if attributes.get("isInternalGroup") and attributes.get("hasAccessToAllBuilds"):
            print(f"{name}: internal group already receives every build")
            continue
        status, body = call("POST", f"/betaGroups/{group['id']}/relationships/builds", {
            "data": [{"type": "builds", "id": build_id}],
        })
        if status >= 300 and status != 409:
            sys.exit(f"Adding the build to {name} failed with HTTP {status}: {json.dumps(body)[:2000]}")
        print(f"{name}: build {build_number} added")
        external = external or not attributes.get("isInternalGroup")

    if external:
        status, body = call("POST", "/betaAppReviewSubmissions", {"data": {
            "type": "betaAppReviewSubmissions",
            "relationships": {"build": {"data": {"type": "builds", "id": build_id}}},
        }})
        if status == 409:
            print("Beta app review: already submitted or not required for this build")
        elif status >= 300:
            sys.exit(f"Submitting for beta app review failed with HTTP {status}: {json.dumps(body)[:2000]}")
        else:
            print("Submitted for beta app review; external testers get it once Apple approves (usually quickly after the first build)")


if __name__ == "__main__":
    main()
