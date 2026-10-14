import assert from "node:assert/strict";
import { test } from "vitest";
import { detectDownloadPlatform, downloadGroups, mobileBetas, playTestingUrl } from "../downloads.ts";

test("desktop hints select current platform packages", () => {
  for (const [userAgent, expected] of [
    ["Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", "macos"],
    ["Mozilla/5.0 (Windows NT 10.0; Win64; x64)", "windows"],
    ["Mozilla/5.0 (X11; Ubuntu; Linux x86_64)", "linux-deb"],
    ["Mozilla/5.0 (X11; Linux x86_64)", "linux"],
    ["unrecognized", null],
  ] as const) {
    assert.equal(detectDownloadPlatform({ userAgent }), expected, userAgent);
  }
  assert.equal(detectDownloadPlatform({ userAgent: "", platform: '"Windows"' }), "windows");
  assert.equal(detectDownloadPlatform({ userAgent: "", platform: "MacIntel" }), "macos");
});

test("phones and tablets get their mobile beta, never a desktop installer", () => {
  for (const [userAgent, expected] of [
    ["Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)", "ios"],
    ["Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X)", "ios"],
    ["Mozilla/5.0 (Linux; Android 15; Pixel 9)", "android"],
    ["Mozilla/5.0 (Linux; Android 10; K)", "android"],
    ["Mozilla/5.0 (X11; CrOS x86_64 16093.0.0)", null],
  ] as const)
    assert.equal(detectDownloadPlatform({ userAgent }), expected, userAgent);
  assert.equal(detectDownloadPlatform({ userAgent: "", platform: '"Android"', mobile: true }), "android");
  assert.equal(detectDownloadPlatform({ userAgent: "Linux", mobile: true }), null);
  // iPadOS sends a Mac user agent; touch points give it away after hydration.
  assert.equal(detectDownloadPlatform({ userAgent: "Macintosh", maxTouchPoints: 5 }), "ios");
  assert.equal(detectDownloadPlatform({ userAgent: "Macintosh", maxTouchPoints: 0 }), "macos");
});

test("Android installs come from the Play closed test, never a sideloaded APK", () => {
  assert.equal(playTestingUrl, "https://play.google.com/apps/testing/chat.caper.android");
  // Google approved the closed test: Android joins the testers group, then opts in on Play.
  assert.equal(mobileBetas.android?.url, "https://groups.google.com/g/caper-testers");
  for (const beta of Object.values(mobileBetas)) if (beta) assert.doesNotMatch(beta.url, /\.apk$/);
});

test("all desktop links use the rolling native release, never legacy previews", () => {
  const base = "https://github.com/joswayski/caper/releases/download/native-latest/";
  assert.deepEqual(
    downloadGroups
      .filter((group) => !["ios", "android"].includes(group.platform))
      .map((group) => group.links.map((link) => link.url)),
    [
      [`${base}Caper-macOS-Apple-Silicon.dmg`, `${base}Caper-macOS-Intel.dmg`],
      [`${base}Caper-Windows-x64-Setup.exe`],
      [`${base}Caper-Linux-x64.deb`, `${base}Caper-Linux-x64.tar.gz`],
    ],
  );
  assert.deepEqual(
    downloadGroups.find((group) => group.platform === "ios")?.links.map((link) => link.url),
    ["https://testflight.apple.com/join/khGQMuYP"],
  );
  assert.deepEqual(
    downloadGroups.find((group) => group.platform === "android")?.links.map((link) => link.url),
    ["https://groups.google.com/g/caper-testers", "https://play.google.com/apps/testing/chat.caper.android"],
  );
});
