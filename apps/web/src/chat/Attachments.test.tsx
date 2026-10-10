import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { MessageAttachments } from "./Attachments.tsx";
import type { ChatAttachment } from "./types.ts";

const file = (id: string, name: string, overrides: Partial<ChatAttachment>): ChatAttachment => ({
  id,
  kind: "image",
  contentType: "image/webp",
  name,
  size: 10,
  url: `https://cdn.test/original/${id}`,
  ...overrides,
});

const files = [
  file("photo", "photo.webp", { width: 800, height: 600, previewUrl: "https://cdn.test/preview/photo" }),
  file("clip", "clip.mp4", { kind: "video", contentType: "video/mp4" }),
  file("gif", "party.mp4", { kind: "video", contentType: "video/mp4", animated: true }),
  file("voice", "voice.m4a", { kind: "audio", contentType: "audio/mp4" }),
  file("notes", "notes.pdf", { kind: "file", contentType: "application/pdf" }),
];

test("sent images open the viewer but keep a real link for new tabs, and videos gain an expand button", () => {
  const markup = renderToStaticMarkup(<MessageAttachments attachments={files} onView={() => {}} />);
  expect(markup).toMatch(
    /<a class="chat-media" href="https:\/\/cdn\.test\/original\/photo" target="_blank" rel="noopener noreferrer"[^>]*aria-haspopup="dialog"/,
  );
  // Videos keep playing inline with their own controls.
  expect(markup).toMatch(/<div class="chat-media chat-video"><video controls=""/);
  expect(markup).toContain('aria-label="Open clip.mp4 in viewer"');
  expect(markup).toContain('aria-label="Open party.mp4 in viewer"');
  expect(markup).not.toContain("Open voice.m4a in viewer");
  expect(markup).not.toContain("Open notes.pdf in viewer");
});

test("without a viewer (the sender's pending row) files keep their links and inline players", () => {
  const markup = renderToStaticMarkup(<MessageAttachments attachments={files} />);
  expect(markup).toContain('href="https://cdn.test/original/photo" target="_blank"');
  expect(markup).toMatch(/<div class="chat-media chat-video"><video controls=""/);
  expect(markup).not.toContain("aria-haspopup");
  expect(markup).not.toContain("in viewer");
});
