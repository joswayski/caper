// Grouped media against the disposable loopback fixture, not live accounts or native devices.
// Start native-parity-fixture.mjs and Vite first. Use a fine-pointer Chromium wrapper.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const web = process.env.MESSAGE_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.MESSAGE_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const directory = mkdtempSync(resolve("apps/web/public/media-test-"));
const browser = (...args) => {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", "media-actions", ...args, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (source) => browser("eval", source).result;
const wait = (source) => browser("wait", "--fn", source);
const control = async (body) => {
  const response = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.equal(response.status, 200);
};

try {
  assert.equal((await (await fetch(`${api}/health`, { headers: { connection: "close" } })).json()).fixture, true);
  await control({ reset: true });
  const otherSpace = await fetch(`${api}/api/spaces`, {
    method: "POST",
    headers: { "content-type": "application/json", cookie: "caper_fixture=owner", connection: "close" },
    body: JSON.stringify({ name: "TEST FIXTURE other space" }),
  });
  assert.equal(otherSpace.status, 201);
  browser("open", "about:blank");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("set", "viewport", "1000", "850", "2");
  browser("open", `${web}/spaces`);
  const row = '[data-message-id="chan00000001m03"]';
  const media = `${row} .chat-media`;
  wait(`!!document.querySelector('${row}.chat-message-grouped') && !document.querySelector('.chat-initial-messages')`);
  assert.equal(evaluate('matchMedia("(hover: hover) and (pointer: fine)").matches'), true);
  const image = `${web}/icons/caper-main-v3-512.png`;
  for (const [layout, width, text] of [
    ["desktop-image-only", 1000, ""],
    ["desktop-caption", 1000, "Image test"],
    ["wide-image-only", 1440, ""],
    ["narrow-image-only", 390, ""],
    ["narrow-caption", 390, "TEST FIXTURE — image with a caption"],
  ]) {
    await control({
      incomingEdit: {
        channelId: "chan00000001",
        messageId: "chan00000001m03",
        text,
        attachments: [
          {
            id: "fixtureimage001",
            kind: "image",
            status: "ready",
            name: "TEST FIXTURE image.png",
            contentType: "image/png",
            size: 1024,
            width: 512,
            height: 512,
            url: image,
          },
        ],
      },
    });
    browser("set", "viewport", String(width), "850", "2");
    browser("reload");
    wait(`!!document.querySelector('${media} img')?.naturalWidth && !document.querySelector('.chat-initial-messages')`);
    browser("hover", media);
    const geometry = evaluate(`(() => {
      const row = document.querySelector('${row}');
      const media = row.querySelector('.chat-media').getBoundingClientRect();
      const body = row.querySelector('.chat-attachments').parentElement;
      const style = getComputedStyle(body);
      const available = body.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
      const actions = [...row.querySelectorAll('.chat-message-actions-trigger, .chat-add-reaction, .chat-reply-message, .chat-reply-thread')]
        .filter(button => getComputedStyle(button).display !== 'none').map(button => button.getBoundingClientRect());
      const overlaps = actions.some(action => media.left < action.right && media.right > action.left && media.top < action.bottom && media.bottom > action.top);
      const inset = 10;
      const corners = [[media.left + inset, media.top + inset], [media.right - inset, media.top + inset]];
      return { overlaps, gap: media.top - Math.max(...actions.map(action => action.bottom)),
        width: media.width, expectedWidth: Math.min(300, available),
        mediaOwnsCorners: corners.every(([x, y]) => document.elementFromPoint(x, y)?.closest('.chat-media') === row.querySelector('.chat-media')) };
    })()`);
    assert.equal(geometry.overlaps, false, `${layout}: actions must not cover media`);
    if (width > 760) assert.ok(geometry.gap >= 4, `${layout}: leave room for the focus ring above media`);
    assert.ok(
      Math.abs(geometry.width - geometry.expectedWidth) < 1,
      `${layout}: toolbar must not squeeze media: ${JSON.stringify(geometry)}`,
    );
    assert.equal(geometry.mediaOwnsCorners, true, `${layout}: media owns its top-corner hit targets`);
    if (artifacts) browser("screenshot", `${artifacts}/${layout}.png`);
    browser("click", media);
    wait('!!document.querySelector(".chat-viewer")');
    browser("press", "Escape");
    wait('!document.querySelector(".chat-viewer")');
    browser("hover", row);
    browser("click", `${row} .chat-message-actions-trigger`);
    wait('!!document.querySelector(".chat-message-actions")');
    browser("press", "Escape");
    wait('!document.querySelector(".chat-message-actions")');
    console.log(`PASS ${layout}: full media width, no toolbar overlap, media hit targets, viewer and actions menu`);
  }

  // A three-colour GIF, one second per frame. The opening history must not
  // briefly animate one image and then replace it with a restarted player.
  writeFileSync(
    join(directory, "animation.gif"),
    Buffer.from(
      "R0lGODlhAQABAPAAALZNMgAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQAZAAAACwAAAAAAQABAAACAkQBACH5BABkAAAALAAAAAABAAEAgGN6QwAAAAICRAEAIfkEAGQAAAAsAAAAAAEAAQCA8e3kAAAAAgJEAQA7",
      "base64",
    ),
  );
  await control({
    incomingEdit: {
      channelId: "chan00000001",
      messageId: "chan00000001m03",
      text: "TEST FIXTURE — GIF playback survives the initial history handoff",
      attachments: [
        {
          id: "fixturegif00001",
          kind: "image",
          status: "ready",
          name: "TEST FIXTURE animation.gif",
          contentType: "image/gif",
          size: 114,
          width: 240,
          height: 120,
          url: `${web}/${basename(directory)}/animation.gif`,
        },
      ],
    },
  });
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "850", "2");
    browser("reload");
    wait(
      `!!document.querySelector('${media} .chat-gif img')?.naturalWidth && !document.querySelector('.chat-initial-messages')`,
    );
    if (width === 390) browser("find", "role", "button", "click", "--name", "Back to Browse", "--exact");
    browser("click", '.space-rail button[title="TEST FIXTURE other space"]');
    wait('!!document.querySelector(".chat-empty")');
    if (width === 390 && !evaluate('document.querySelector(".spaces-room").classList.contains("navigation-open")'))
      browser("find", "role", "button", "click", "--name", "Back to Browse", "--exact");
    evaluate(`(() => {
      const players = new Set();
      window.gifPlayback = { players: 0, samples: 0, done: false };
      let first;
      const sample = () => {
        const images = [...document.querySelectorAll('${media} .chat-gif img')]
          .filter(image => image.naturalWidth && image.checkVisibility({ visibilityProperty: true }));
        if (images.length) {
          first ??= performance.now();
          for (const image of images) players.add(image);
          window.gifPlayback.players = players.size;
          window.gifPlayback.samples++;
        }
        if (first && performance.now() - first > 1200) window.gifPlayback.done = true;
        else requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    })()`);
    browser("click", '.space-rail button[title="Fixture Studio"]');
    if (width === 390) {
      wait('!!document.querySelector(\'.space-rail-item[data-active="true"] button[title="Fixture Studio"]\')');
      if (evaluate('document.querySelector(".spaces-room").classList.contains("navigation-open")'))
        browser("find", "role", "button", "click", "--name", "general", "--exact");
    }
    wait("window.gifPlayback.done");
    const playback = evaluate("window.gifPlayback");
    assert.ok(playback.samples > 10, "Observe playback across multiple painted frames");
    assert.equal(playback.players, 1, `${width}px: visible GIF playback must not restart in a replacement image`);
    if (artifacts) browser("screenshot", `${artifacts}/gif-space-switch-${width}.png`);
    console.log(`PASS ${width}px: one continuous GIF player across space navigation (${playback.samples} frames)`);
  }
  browser("set", "media", "reduced-motion");
  assert.equal(evaluate(`getComputedStyle(document.querySelector('${media} .chat-gif img')).visibility`), "hidden");
  assert.equal(evaluate(`getComputedStyle(document.querySelector('${media} .chat-gif canvas')).display`), "block");
  console.log("PASS reduced motion: GIF stays on its canvas still");
} finally {
  browser("close");
  rmSync(directory, { recursive: true, force: true });
}
