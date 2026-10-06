import assert from "node:assert/strict";
import test from "node:test";
import { detectDownloadPlatform, downloads, intelMacDownload } from "../downloads.ts";

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

test("mobile, tablets and ChromeOS never receive a desktop installer", () => {
  for (const userAgent of [
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)",
    "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X)",
    "Mozilla/5.0 (Linux; Android 15; Pixel 9)",
    "Mozilla/5.0 (X11; CrOS x86_64 16093.0.0)",
  ]) assert.equal(detectDownloadPlatform({ userAgent }), null, userAgent);
  assert.equal(detectDownloadPlatform({ userAgent: "Linux", mobile: true }), null);
  assert.equal(detectDownloadPlatform({ userAgent: "Macintosh", maxTouchPoints: 5 }), null);
  assert.equal(detectDownloadPlatform({ userAgent: "Macintosh", maxTouchPoints: 0 }), "macos");
});

test("all desktop links use the rolling native release, never legacy previews", () => {
  const base = "https://github.com/joswayski/caper/releases/download/native-latest/";
  assert.equal(downloads.macos.url, `${base}Caper-macOS-Apple-Silicon.dmg`);
  assert.equal(intelMacDownload, `${base}Caper-macOS-Intel.dmg`);
  assert.equal(downloads.windows.url, `${base}Caper-Windows-x64-Setup.exe`);
  assert.equal(downloads["linux-deb"].url, `${base}Caper-Linux-x64.deb`);
  assert.equal(downloads.linux.url, "https://github.com/joswayski/caper/releases/tag/native-latest");
});
