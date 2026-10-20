import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import { fit, MessageEmbeds, playerUrl } from "../chat/Embeds.tsx";
import { embedOf, embedsOf, isChatEmbedsEvent, previewLink, withEmbeds, type ChatEmbedsEvent } from "../chat/embeds.ts";
import { ChatTimeline } from "../chat/timeline.ts";
import { isChatMessage, type ChatMessage } from "../chat/types.ts";

const tweet = {
  version: 1,
  url: "https://x.com/jack/status/20",
  site: { name: "X", domain: "x.com" },
  blocks: [
    { type: "author", name: "jack", handle: "@jack", url: "https://x.com/jack", icon: "https://embed.test/m/s/i?u=a" },
    { type: "text", text: "just setting up my twttr https://example.com" },
    {
      type: "video",
      url: "https://video.twimg.com/a.mp4",
      poster: "https://embed.test/m/s/i?u=b",
      width: 1280,
      height: 720,
    },
    { type: "footer", text: "309K likes", timestamp: "2006-03-21T20:50:14.000Z" },
  ],
};

function message(seq: string, extra: Partial<ChatMessage> = {}, embeds?: unknown[]): ChatMessage {
  return {
    id: `m${seq}`,
    channelId: "general",
    seq,
    author: { id: "a", name: "Ann", isGuest: false },
    content: { version: 1, type: "text", text: "see https://x.com/jack/status/20", ...(embeds ? { embeds } : {}) },
    createdAt: "2026-10-20T12:00:00Z",
    clientMessageId: `c${seq}`,
    ...extra,
  };
}

const event = (messageId: string, revision: number, embeds: unknown[]): ChatEmbedsEvent => ({
  type: "message.embeds",
  schemaVersion: 1,
  channelId: "general",
  messageId,
  revision,
  embeds,
});

test("cards keep known, well-formed blocks and drop everything else", () => {
  assert.deepEqual(embedOf(tweet), tweet);
  const mixed = embedOf({
    ...tweet,
    blocks: [
      { type: "hologram", text: "from the future" },
      { type: "author", name: "jack", icon: "javascript:alert(1)" },
      { type: "images", items: [{ url: "data:image/png;base64,AA" }, { url: "https://embed.test/i.png", width: 2 }] },
      { type: "player", provider: "flash", id: "x" },
      { type: "text", text: "kept" },
    ],
  });
  assert.deepEqual(mixed?.blocks, [
    { type: "images", items: [{ url: "https://embed.test/i.png", width: 2 }], thumbnail: false },
    { type: "text", text: "kept" },
  ]);
  assert.equal(embedOf({ ...tweet, url: "javascript:alert(1)" }), null);
  assert.equal(embedOf({ ...tweet, version: 2 }), null);
  assert.equal(embedOf({ ...tweet, blocks: [{ type: "hologram" }] }), null);
  // One bad card never hides the others or the message.
  const withBad = message("1", {}, [tweet, { nope: true }]);
  assert.ok(isChatMessage(withBad));
  assert.equal(embedsOf(withBad).length, 1);
  assert.deepEqual(embedsOf({ ...withBad, deletedAt: "2026-10-20T12:01:00Z" }), []);
});

test("cards apply only to the revision they were made for", () => {
  const original = message("1");
  assert.deepEqual(withEmbeds(original, 1, [tweet]).content.embeds, [tweet]);
  assert.equal(withEmbeds(original, 2, [tweet]), original);
  const edited = message("1", { revision: 2, editedAt: "x", editSeq: "4" }, [tweet]);
  assert.equal(withEmbeds(edited, 2, []).content.embeds, undefined);
  assert.ok(isChatEmbedsEvent(event("m1", 1, [tweet])));
  assert.ok(!isChatEmbedsEvent({ ...event("m1", 1, []), revision: 0 }));
  assert.ok(!isChatEmbedsEvent({ ...event("m1", 1, []), embeds: "nope" }));
});

test("the timeline takes cards live, after a gap, and from a refetch of the same revision", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("1")], "1");
  assert.equal(timeline.applyEmbeds(event("m1", 1, [tweet])), true);
  assert.deepEqual(timeline.messages[0].content.embeds, [tweet]);
  assert.equal(timeline.applyEmbeds(event("m1", 2, [])), false);
  // Cards for a message not loaded yet wait for it.
  assert.equal(timeline.applyEmbeds(event("m2", 1, [tweet])), false);
  timeline.applyEvent(message("2"));
  assert.deepEqual(timeline.messages[1].content.embeds, [tweet]);
  // A replay of the stored message brings cards the live event never delivered.
  const later = new ChatTimeline();
  later.reset([message("1")], "1");
  later.applyEvent(message("1", {}, [tweet]));
  assert.deepEqual(later.messages[0].content.embeds, [tweet]);
});

test("the composer previews the first link the API would preview", () => {
  assert.equal(previewLink("look www.example.com/a and https://x.com/b"), "https://www.example.com/a");
  assert.equal(previewLink("<https://quiet.example/> then https://loud.example/"), "https://loud.example/");
  assert.equal(previewLink("no links here"), undefined);
});

test("cards render as plain text with external links and players only after Play", () => {
  const html = renderToStaticMarkup(
    createElement(MessageEmbeds, {
      embeds: [
        embedOf(tweet)!,
        embedOf({
          version: 1,
          url: "https://youtu.be/dQw4w9WgXcQ",
          site: { name: "YouTube", domain: "youtu.be" },
          blocks: [
            { type: "title", text: "<script>alert(1)</script>" },
            { type: "player", provider: "youtube", id: "dQw4w9WgXcQ", poster: "https://embed.test/p.jpg" },
          ],
        })!,
      ],
    }),
  );
  assert.match(html, /<article class="chat-embed" aria-label="X: jack">/);
  assert.match(
    html,
    /href="https:\/\/x\.com\/jack\/status\/20" target="_blank" rel="noopener noreferrer nofollow ugc"/,
  );
  assert.match(html, /<video src="https:\/\/video\.twimg\.com\/a\.mp4"/);
  assert.ok(html.includes("&lt;script&gt;alert(1)&lt;/script&gt;"));
  assert.ok(!html.includes("<iframe"), "no third-party player before Play");
  assert.match(html, /aria-label="Play &lt;script&gt;/);
});

test("media keeps its proportions within 400×300, and unknown sizes get full width", () => {
  assert.deepEqual(fit(1920, 1080), { width: 400, aspectRatio: "1920 / 1080" });
  assert.deepEqual(fit(576, 1024), { width: 169, aspectRatio: "576 / 1024" });
  assert.deepEqual(fit(200, 100), { width: 200, aspectRatio: "200 / 100" });
  assert.deepEqual(fit(), { width: 400, aspectRatio: "16 / 9" });
});

test("players load each provider's own embed page", () => {
  const player = { type: "player" as const, id: "dQw4w9WgXcQ", start: 90 };
  assert.equal(
    playerUrl({ ...player, provider: "youtube" }, "caper.chat"),
    "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&playsinline=1&start=90",
  );
  assert.equal(
    playerUrl({ ...player, provider: "vimeo", id: "1084537" }, "caper.chat"),
    "https://player.vimeo.com/video/1084537?autoplay=1",
  );
  assert.equal(
    playerUrl({ ...player, provider: "twitch", id: "clip:Funny-1" }, "caper.chat"),
    "https://clips.twitch.tv/embed?clip=Funny-1&parent=caper.chat&autoplay=true",
  );
  assert.equal(
    playerUrl({ ...player, provider: "twitch", id: "channel:streamer" }, "caper.chat"),
    "https://player.twitch.tv/?channel=streamer&parent=caper.chat&autoplay=true",
  );
});
