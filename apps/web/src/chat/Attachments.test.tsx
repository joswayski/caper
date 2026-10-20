import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { DraftAttachments, MessageAttachments, type DraftAttachment } from "./Attachments.tsx";
import Composer from "./Composer.tsx";
import { ReplyContext } from "./Replies.tsx";
import type { ChatAttachment } from "./types.ts";

test("reply previews stay bounded and never load concealed media or video originals", () => {
  const photo: ChatAttachment = {
    id: "photo",
    kind: "image",
    contentType: "image/png",
    name: "Photo",
    size: 10,
    url: "https://cdn.test/photo",
    previewUrl: "https://cdn.test/preview",
  };
  const render = (files: ChatAttachment[], hidden = false, deleted = false) =>
    renderToStaticMarkup(
      <ReplyContext
        motionKey="reply"
        reference={{
          id: "source",
          author: { id: "me", name: "Me", isGuest: false },
          text: "Photo",
          hasAttachments: true,
          attachments: files,
          deleted,
        }}
        hidden={hidden}
        onOpen={() => {}}
      />,
    );
  const markup = render([
    photo,
    {
      ...photo,
      id: "spoiler",
      spoiler: true,
      url: "https://cdn.test/secret",
      previewUrl: "https://cdn.test/secret-preview",
    },
    { ...photo, id: "video", kind: "video", previewUrl: undefined, url: "https://cdn.test/video" },
    { ...photo, id: "overflow" },
  ]);
  expect(markup).toContain('src="https://cdn.test/preview"');
  expect(markup).not.toContain("https://cdn.test/secret");
  expect(markup).not.toContain("https://cdn.test/video");
  expect(markup).toContain("+1");
  for (const file of [
    { ...photo, unavailable: true },
    { ...photo, status: "processing" as const },
    { ...photo, status: "failed" as const },
  ])
    expect(render([file])).not.toContain("https://cdn.test/");
  expect(render([photo], true)).not.toContain("https://cdn.test/");
  expect(render([photo], false, true)).not.toContain("https://cdn.test/");
});

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

// Attach stays enabled at the limit: its drawer shows the full capacity line.
test("a full tray explains the attachment limit", () => {
  const drafts = Array.from({ length: 10 }, (_, index) => ({
    key: `${index}`,
    name: "a.txt",
    sourceSize: 1,
    progress: 0,
  }));
  const attach = attachButton(true, drafts).match(/<button[^>]*aria-label="Attach files"[^>]*>/)?.[0];
  expect(attach).not.toContain('disabled=""');
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

test.each(["processing", "ready"] as const)("own photos keep their local preview when %s", (status) => {
  const photo = { ...files[0], status };
  const markup = renderToStaticMarkup(
    <MessageAttachments
      attachments={[photo]}
      localPreviews={{ photo: { url: "blob:local-photo", video: false } }}
      progress={{ photo: 37 }}
      onView={() => {}}
    />,
  );
  expect(markup).toContain('src="blob:local-photo"');
  expect(markup).not.toContain('src="https://cdn.test/preview/photo"');
  if (status === "processing") {
    expect(markup).toContain("Processing… 37%");
    expect(markup).not.toMatch(/<a\b[^>]*\bhref=/);
    expect(markup).not.toContain("aria-haspopup");
  } else {
    expect(markup).toContain('href="https://cdn.test/original/photo"');
    expect(markup).toContain('aria-haspopup="dialog"');
    expect(markup).not.toContain("Processing…");
  }
});

test.each([{ status: "failed" as const }, { unavailable: true }])(
  "a local preview never hides a removed or failed file: %j",
  (state) => {
    const markup = renderToStaticMarkup(
      <MessageAttachments
        attachments={[{ ...files[0], ...state }]}
        localPreviews={{ photo: { url: "blob:local-photo", video: false } }}
      />,
    );
    expect(markup).not.toContain("blob:local-photo");
    expect(markup).not.toContain("<img");
    expect(markup).toContain(state.status === "failed" ? "Couldn’t process this file" : "File removed");
  },
);

test("spoilers show only still masked previews, never players, links, or filenames", () => {
  const hidden = [
    ...files,
    file("processing", "secret.png", { status: "processing" }),
    file("original", "secret.gif", { contentType: "image/gif" }),
    file("poster", "secret.mp4", { kind: "video", animated: true, previewUrl: "https://cdn.test/poster" }),
    file("removed", "removed.png", { unavailable: true }),
    file("failed", "failed.png", { status: "failed" }),
  ].map((attachment) => ({
    ...attachment,
    spoiler: true,
  }));
  const markup = renderToStaticMarkup(
    <MessageAttachments attachments={hidden} localPreviews={{ processing: { url: "blob:secret", video: false } }} />,
  );
  expect(markup.match(/aria-label="Reveal spoiler attachment"/g)).toHaveLength(hidden.length);
  expect(markup).not.toMatch(/<(?:video|audio|a)\b/);
  for (const attachment of hidden) expect(markup).not.toContain(attachment.name);
  expect(markup.match(/class="chat-spoiler-preview"/g)).toHaveLength(4);
  expect(markup.match(/data-still="true"/g)).toHaveLength(4);
  expect(markup).toContain('src="https://cdn.test/preview/photo"');
  expect(markup).toContain('src="blob:secret"');
  expect(markup).toContain('src="https://cdn.test/original/original"');
  expect(markup).toContain('src="https://cdn.test/poster"');
  for (const id of ["photo", "clip", "gif", "voice", "notes", "removed", "failed"])
    expect(markup).not.toContain(`https://cdn.test/original/${id}`);
  const mixed = renderToStaticMarkup(<MessageAttachments attachments={[hidden[0], files[1]]} />);
  expect(mixed).toContain("Reveal spoiler attachment");
  expect(mixed).toContain('aria-label="clip.mp4"');
});

test("a video with a stored size reserves its shape without fixing the responsive height", () => {
  const sized = file("sized", "sized.mp4", { kind: "video", contentType: "video/mp4", width: 1080, height: 1920 });
  const markup = renderToStaticMarkup(<MessageAttachments attachments={[sized]} />);
  const ratio = markup.match(
    /<div class="chat-media chat-video" style="width:\d+px;aspect-ratio:([\d.]+)"><video/,
  )?.[1];
  expect(Number(ratio)).toBeCloseTo(9 / 16, 2);
});

test("the media group keeps processing, failed and removed slots without grouping audio or file controls", () => {
  const attachments = [
    files[0],
    files[4],
    { ...files[1], status: "processing" as const },
    file("failed", "failed.webp", { status: "failed" }),
    files[3],
    file("removed", "removed.webp", { unavailable: true }),
  ];
  const markup = renderToStaticMarkup(<MessageAttachments attachments={attachments} />);
  expect(markup).toContain('role="group" aria-label="4 attachments"');
  expect(markup).toContain('aria-label="clip.mp4: Processing…"');
  expect(markup).toContain("Processing failed");
  expect(markup).toContain("File removed");
  expect(markup).not.toContain('href="https://cdn.test/original/removed"');
  expect(markup.indexOf("removed.webp")).toBeLessThan(markup.indexOf("notes.pdf"));
  expect(markup.indexOf("notes.pdf")).toBeLessThan(markup.indexOf("voice.m4a"));
  const ready = renderToStaticMarkup(
    <MessageAttachments
      attachments={attachments.map((attachment) => ({ ...attachment, status: "ready", unavailable: false }))}
    />,
  );
  expect(ready).toContain('role="group" aria-label="4 attachments"');
  expect(ready).toContain('href="https://cdn.test/original/removed"');
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
  expect(visibleText).toContain("Uploading 40%");
  expect(visibleText).toContain("Retry upload");
  expect(visibleText).toContain("1 file failed to upload. Send will retry it, or you can remove it.");
  expect(markup).toContain('title="Upload failed"');
  expect(markup).toContain('src="https://cdn.test/preview/photo"');

  const retrying = drafts.map((draft) => ({ ...draft, error: undefined }));
  const uploading = renderToStaticMarkup(<DraftAttachments drafts={retrying} />);
  expect(uploading).not.toContain('role="status"');
  expect(uploading).not.toContain("Your message won’t send until every file is ready.");
  expect(uploading).toContain("Uploading 40%");
  expect(uploading).toContain("Preparing…");
  expect(uploading).not.toContain("failed to upload");
  const staged = renderToStaticMarkup(
    <DraftAttachments drafts={retrying.map((draft) => ({ ...draft, staged: true }))} />,
  );
  expect(staged).not.toContain('role="status"');
});

test("a draft's Spoiler, Edit and Remove share one bar, Edit only for files the editor opens", () => {
  const drafts: DraftAttachment[] = [
    {
      key: "photo",
      name: "photo.jpg",
      localUrl: "blob:photo",
      localKind: "image",
      editable: "image",
      sourceSize: 1,
      progress: 0,
    },
    { key: "notes", name: "notes.pdf", localUrl: "blob:notes", sourceSize: 1, progress: 0 },
  ];
  const editing = { source: () => new Blob(), save: () => {} };
  const markup = renderToStaticMarkup(<DraftAttachments drafts={drafts} onRemove={() => {}} editing={editing} />);
  expect(markup).toMatch(
    /<div class="chat-draft-actions"><button[^>]*aria-label="Edit photo\.jpg"[^>]*>.*?<\/button><button[^>]*aria-label="Remove photo\.jpg"/,
  );
  expect(markup).toMatch(/<div class="chat-draft-actions"><button[^>]*aria-label="Remove notes\.pdf"/);
  expect(markup).not.toContain('aria-label="Edit notes.pdf"');
  const spoilers = renderToStaticMarkup(
    <DraftAttachments drafts={drafts} onRemove={() => {}} onToggleSpoiler={() => {}} editing={editing} />,
  );
  expect(spoilers).toMatch(
    /<div class="chat-draft-actions"><button[^>]*aria-label="Spoiler attachment: photo\.jpg"[^>]*>.*?<\/button><button[^>]*aria-label="Edit photo\.jpg"[^>]*>.*?<\/button><button[^>]*aria-label="Remove photo\.jpg"/,
  );
  // While sending nothing can be removed or edited, so there is no bar.
  expect(renderToStaticMarkup(<DraftAttachments drafts={drafts} editing={editing} />)).not.toContain(
    "chat-draft-actions",
  );
});

test("a draft video keeps its poster and spoiler state, and cannot be removed while sending", () => {
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
    for (const spoiler of [true, false]) {
      const markup = renderToStaticMarkup(
        <DraftAttachments drafts={[{ ...draft, spoiler }]} onRemove={removable ? () => {} : undefined} />,
      );
      expect(markup).toContain('src="https://cdn.test/preview/clip"');
      expect(markup.includes('aria-label="Video"')).toBe(!spoiler);
      expect(markup.includes('data-spoiler="true"')).toBe(spoiler);
      expect(markup.includes(">Spoiler</span>")).toBe(spoiler);
      expect(markup).toContain('aria-label="Preview clip.mp4"');
      expect(markup.includes('aria-label="Remove clip.mp4"')).toBe(removable);
      expect(markup).not.toContain("Remove attachment");
    }
  }
});
