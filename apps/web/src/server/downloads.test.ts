import assert from "node:assert/strict";
import { createMemoryHistory, createRootRoute, createRouter, RouterContextProvider } from "@tanstack/react-router";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import {
  detectDownloadPlatform,
  downloads,
  intelMacDownload,
  linuxDownloads,
  mobileBetas,
  playTestingUrl,
} from "../downloads.ts";
import Home from "../pages/Home";

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
  assert.equal(downloads.macos.url, `${base}Caper-macOS-Apple-Silicon.dmg`);
  assert.equal(intelMacDownload, `${base}Caper-macOS-Intel.dmg`);
  assert.equal(downloads.windows.url, `${base}Caper-Windows-x64-Setup.exe`);
  assert.deepEqual(
    linuxDownloads.map((download) => download.url),
    [`${base}Caper-Linux-x64.deb`, `${base}Caper-Linux-x64.tar.gz`],
  );
});

test("each homepage offers only its own platform: direct links except for the Linux package picker", () => {
  const base = "https://github.com/joswayski/caper/releases/download/native-latest/";
  const repository = "https://github.com/joswayski/caper";
  for (const [platform, hrefs] of [
    ["macos", [`${base}Caper-macOS-Apple-Silicon.dmg`, repository]],
    ["windows", [`${base}Caper-Windows-x64-Setup.exe`, repository]],
    ["ios", ["https://testflight.apple.com/join/khGQMuYP", repository]],
    [
      "android",
      [
        "https://groups.google.com/g/caper-testers",
        "https://play.google.com/apps/testing/chat.caper.android",
        repository,
      ],
    ],
    ["linux", [repository]],
    ["linux-deb", [repository]],
    [null, [repository]],
  ] as const) {
    const router = createRouter({
      routeTree: createRootRoute(),
      history: createMemoryHistory({ initialEntries: ["/"] }),
      isServer: true,
    });
    const markup = renderToStaticMarkup(
      createElement(RouterContextProvider, {
        router,
        children: createElement(Home, {
          account: null,
          initialNow: 0,
          latestChanges: [],
          downloadPlatform: platform,
        }),
      }),
    );
    const actions = markup.slice(
      markup.indexOf('<div class="app-downloads">'),
      markup.indexOf('<p class="experimental-note">'),
    );
    assert.deepEqual(
      [...actions.matchAll(/href="([^"]+)"/g)].map((match) => match[1]),
      hrefs,
      String(platform),
    );
    assert.equal(
      actions.includes('aria-haspopup="dialog"'),
      platform === "linux" || platform === "linux-deb",
      String(platform),
    );
    assert.doesNotMatch(actions, /<select|available for/i);
    assert.equal(actions.includes("Download for Linux"), platform === "linux" || platform === "linux-deb");
  }
});

test("the homepage join link skips the spaces loader until the visitor has a complete account", () => {
  for (const [account, destination] of [
    [null, "/login"],
    [undefined, "/login"],
    [{ id: "me", username: null, displayName: null }, "/profile"],
    [{ id: "me", username: "river", displayName: null }, "/profile"],
    [{ id: "me", username: "river", displayName: "River" }, "/spaces"],
  ] as const) {
    const router = createRouter({
      routeTree: createRootRoute(),
      history: createMemoryHistory({ initialEntries: ["/"] }),
      isServer: true,
    });
    const markup = renderToStaticMarkup(
      createElement(RouterContextProvider, {
        router,
        children: createElement(Home, {
          account,
          initialNow: 0,
          latestChanges: [],
          downloadPlatform: null,
        }),
      }),
    );
    const join = markup.match(/<a\b[^>]*aria-label="Join Caper"[^>]*>/)?.[0];
    assert.ok(join);
    assert.ok(join.includes(`href="${destination}"`), join);
  }
});
