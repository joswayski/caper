// Grouped media against the disposable loopback fixture, not live accounts or native devices.
// Start native-parity-fixture.mjs and Vite first. Use a fine-pointer Chromium wrapper.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const web = process.env.MESSAGE_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.MESSAGE_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
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
} finally {
  browser("close");
}
