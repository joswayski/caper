import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { DraftAttachments, MessageAttachments, type DraftAttachment } from "./Attachments.tsx";
import Composer from "./Composer.tsx";
import type { ChatAttachment } from "./types.ts";

const attachButton = (enabled: boolean, drafts: DraftAttachment[] = []) =>
  renderToStaticMarkup(
    <Composer
      channelName="general"
      direct={false}
      disabled={false}
      identityReady
      sending={false}
      sendRejected={false}
      onSend={() => undefined}
      onTyping={() => {}}
      onResize={() => {}}
      onDraftPresence={() => {}}
      attachments={{
        enabled,
        disabledReason: "File uploads are unavailable",
        drafts,
        onAdd: () => {},
        onRemove: () => {},
        onRetry: () => {},
      }}
    />,
  );

test.each([false, true])("the attachment slot stays present while availability changes to %s", (enabled) => {
  const markup = attachButton(enabled);
  expect(markup).toContain('data-attach="true"');
  const attach = markup.match(/<button[^>]*aria-label="Attach files"[^>]*>/)?.[0];
  expect(attach).toBeDefined();
  expect(attach!.includes('disabled=""')).toBe(!enabled);
  expect(attach).toContain(`title="${enabled ? "Attach files" : "File uploads are unavailable"}"`);
});

test("a full tray explains the attachment limit", () => {
  const drafts = Array.from({ length: 10 }, (_, index) => ({
    key: `${index}`,
    name: "a.txt",
    sourceSize: 1,
    progress: 0,
  }));
  const attach = attachButton(true, drafts).match(/<button[^>]*aria-label="Attach files"[^>]*>/)?.[0];
  expect(attach).toContain('disabled=""');
  expect(attach).toContain('title="You can attach up to 10 files."');
});

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
  // Videos keep playing inline with their own controls; one with no stored size keeps its own shape.
  expect(markup).toMatch(/<div class="chat-media chat-video intrinsic"><video controls=""/);
  expect(markup).toContain('aria-label="Open clip.mp4 in viewer"');
  expect(markup).toContain('aria-label="Open party.mp4 in viewer"');
  expect(markup).not.toContain("Open voice.m4a in viewer");
  expect(markup).not.toContain("Open notes.pdf in viewer");
});

test("without a viewer (the sender's pending row) files keep their links and inline players", () => {
  const markup = renderToStaticMarkup(<MessageAttachments attachments={files} />);
  expect(markup).toContain('href="https://cdn.test/original/photo" target="_blank"');
  expect(markup).toMatch(/<div class="chat-media chat-video intrinsic"><video controls=""/);
  expect(markup).not.toContain("aria-haspopup");
  expect(markup).not.toContain("in viewer");
});

test("a video with a stored size reserves its shape without fixing the responsive height", () => {
  const sized = file("sized", "sized.mp4", { kind: "video", contentType: "video/mp4", width: 1080, height: 1920 });
  const markup = renderToStaticMarkup(<MessageAttachments attachments={[sized]} />);
  const ratio = markup.match(
    /<div class="chat-media chat-video" style="width:\d+px;aspect-ratio:([\d.]+)"><video/,
  )?.[1];
  expect(Number(ratio)).toBeCloseTo(9 / 16, 2);
});

test("draft files can be previewed before uploading finishes, with removal separate from preview", () => {
  const drafts: DraftAttachment[] = [
    { key: "preparing", name: "photo.jpg", localUrl: "blob:photo", localKind: "image", sourceSize: 14000, progress: 0 },
    {
      key: "uploading",
      name: "clip.mp4",
      localUrl: "blob:clip",
      localKind: "video",
      sourceSize: 80000,
      storedSize: 60000,
      progress: 0.4,
    },
    {
      key: "failed",
      name: "failed.png",
      localUrl: "blob:failed",
      localKind: "image",
      sourceSize: 50000,
      progress: 0,
      error: "Upload failed",
    },
    { key: "restored", name: "photo.webp", sourceSize: 10, progress: 1, attachment: files[0] },
    { key: "document", name: "notes.pdf", localUrl: "blob:notes", sourceSize: 1000, progress: 0 },
    { key: "missing", name: "missing.zip", sourceSize: 1000, progress: 0 },
  ];
  const markup = renderToStaticMarkup(<DraftAttachments drafts={drafts} onRemove={() => {}} onRetry={() => {}} />);
  expect(markup).toContain('aria-label="Retry uploading failed.png"');
  for (const name of ["photo.jpg", "clip.mp4", "failed.png", "photo.webp"]) {
    expect(markup).toContain(`aria-label="Preview ${name}" aria-haspopup="dialog"`);
    expect(markup).toMatch(new RegExp(`</button>.*?<button[^>]*aria-label="Remove ${name.replaceAll(".", "\\.")}"`));
  }
  expect(markup).toContain('href="blob:notes" target="_blank" rel="noopener noreferrer" aria-label="Open notes.pdf"');
  expect(markup).not.toContain('aria-label="Preview missing.zip"');
  expect(markup).not.toContain('aria-label="Open missing.zip"');
  // Filenames remain in accessible action labels, not in the compact tiles.
  const visibleText = markup.replace(/<[^>]*>/g, "");
  for (const draft of drafts) expect(visibleText).not.toContain(draft.name);
  expect(visibleText).toContain("Preparing…");
  expect(visibleText).toContain("40%");
  expect(visibleText).toContain("Failed · Retry");
  expect(markup).toContain('title="Upload failed"');
  expect(markup).toContain('src="https://cdn.test/preview/photo"');
});

test("a draft video uses its poster, keeps its video identity, and cannot be removed while sending", () => {
  const draft: DraftAttachment = {
    key: "clip",
    name: "clip.mp4",
    sourceSize: 80000,
    progress: 1,
    localUrl: "blob:clip",
    localKind: "video",
    attachment: { ...files[1], previewUrl: "https://cdn.test/preview/clip" },
  };
  for (const removable of [true, false]) {
    const markup = renderToStaticMarkup(
      <DraftAttachments drafts={[draft]} onRemove={removable ? () => {} : undefined} />,
    );
    expect(markup).toContain('src="https://cdn.test/preview/clip"');
    expect(markup).toContain('aria-label="Video"');
    expect(markup).toContain('aria-label="Preview clip.mp4"');
    expect(markup.includes('aria-label="Remove clip.mp4"')).toBe(removable);
    expect(markup).not.toContain("Remove attachment");
  }
});
