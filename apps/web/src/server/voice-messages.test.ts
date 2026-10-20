import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";
import { DraftAttachments, MessageAttachments } from "../chat/Attachments.tsx";
import ProfileComposer, { voiceCommand } from "../chat/ProfileComposer.tsx";
import { ChatTimeline } from "../chat/timeline.ts";
import {
  attachmentsOf,
  isChatAttachment,
  type ChatAttachment,
  type ChatAttachmentsEvent,
  type ChatMessage,
} from "../chat/types.ts";
import { AttachmentUpload, voicePreparedFile, type UploadTransport } from "../chat/uploads.ts";
import { VOICE_BARS, VoiceTranscript } from "../chat/VoiceMessage.tsx";
import {
  attachmentLabel,
  attachmentsExcerpt,
  decodeWaveform,
  encodeWaveform,
  formatVoiceDuration,
  isVoiceAttachment,
  barFill,
  barsFor,
  getVoiceSendOnStop,
  loudness,
  resampleWaveform,
  setVoiceSendOnStop,
  smoothLevel,
  spokenDuration,
  transcriptView,
  voiceContentType,
  voiceFilename,
  voiceRecordingType,
  waveformFromLevels,
  WAVEFORM_SLICES,
  type VoiceRecording,
} from "../chat/voice.ts";

const voiceFile = (overrides: Partial<ChatAttachment> = {}): ChatAttachment => ({
  id: "v1",
  kind: "audio",
  contentType: "audio/mp4",
  name: "Voice message.m4a",
  size: 31_337,
  durationMs: 4_200,
  url: "https://cdn.test/v1.m4a",
  voice: true,
  waveform: encodeWaveform(Uint8Array.from({ length: 64 }, (_, index) => (index * 4) % 256)),
  transcript: { status: "pending" },
  ...overrides,
});

test("the waveform is 64 slices of RMS loudness, smoothed and scaled so the loudest is 255", () => {
  // Half quiet (−40 dB RMS), half loud (−6 dB RMS), sampled at a steady rate.
  const levels = Array.from({ length: 640 }, (_, index) => (index < 320 ? 0.01 : 0.5));
  const bytes = waveformFromLevels(levels);
  assert.equal(bytes.length, WAVEFORM_SLICES);
  assert.equal(Math.max(...bytes), 255, "normalised to this recording");
  // Perceptual scale: −40 dB is 0.2 and −6 dB about 0.88, so quiet sits low but visible.
  assert.equal(bytes[0], Math.round((0.2 / loudness(0.5)) * 255));
  assert.ok(bytes[0] > 40 && bytes[0] < 70);
  assert.equal(bytes[63], 255);
  // The 3-tap average softens the step instead of jumping in one slice.
  assert.ok(bytes[31] > bytes[30] && bytes[31] < bytes[32] && bytes[32] < bytes[33]);
  // RMS of the slice, not its peak: a 100-sample slice with one full-scale click
  // is −20 dB (0.6), where a peak would have read 0 dB.
  const click = Array.from({ length: 6_400 }, (_, index) => (index === 5 ? 1 : 0));
  assert.equal(waveformFromLevels(click, 64)[0], 255);
  assert.ok(Math.abs(loudness(Math.sqrt(1 / 100)) - 0.6) < 1e-9);
  assert.deepEqual(Array.from(waveformFromLevels(Array(100).fill(0))), Array(64).fill(0), "silence stays flat");
  assert.deepEqual(Array.from(waveformFromLevels([])), Array(64).fill(0));
  // Fewer samples than slices still fill every slice.
  assert.equal(waveformFromLevels([0.1, 0.5]).length, 64);
  assert.equal(loudness(0), 0);
  assert.equal(loudness(1), 1);
  assert.equal(loudness(0.001), 0, "−60 dB is below the −50 dB floor");
  assert.ok(Math.abs(loudness(10 ** (-25 / 20)) - 0.5) < 1e-9, "−25 dB is halfway");
});

test("waveforms travel as standard base64 and decode only within the server's 1–256 bytes", () => {
  const bytes = Uint8Array.from({ length: 64 }, (_, index) => index * 4);
  const encoded = encodeWaveform(bytes);
  assert.equal(encoded, Buffer.from(bytes).toString("base64"));
  assert.ok(encoded.endsWith("="), "padded");
  assert.deepEqual(decodeWaveform(encoded), bytes);
  assert.deepEqual(decodeWaveform(Buffer.from([7]).toString("base64")), Uint8Array.of(7));
  assert.equal(decodeWaveform(Buffer.alloc(257).toString("base64")), undefined);
  assert.equal(decodeWaveform(""), undefined);
  assert.equal(decodeWaveform("not base64!"), undefined);
  assert.equal(decodeWaveform(undefined), undefined);
  assert.equal(decodeWaveform(42), undefined);
});

test("any waveform length resamples by linear interpolation to the bars that fit", () => {
  const bytes = Uint8Array.from([0, 255, 51]);
  assert.deepEqual(resampleWaveform(bytes, 3), [0, 1, 0.2]);
  assert.deepEqual(resampleWaveform(bytes, 5), [0, 0.5, 1, 0.6, 0.2], "stretching interpolates, never steps");
  assert.deepEqual(resampleWaveform(Uint8Array.from([0, 255]), 2), [0, 1]);
  assert.deepEqual(resampleWaveform(Uint8Array.from([255]), 3), [1, 1, 1]);
  assert.equal(resampleWaveform(new Uint8Array(64).fill(255), VOICE_BARS).length, VOICE_BARS);
  assert.deepEqual(resampleWaveform(undefined, 3), [0, 0, 0], "a missing waveform is flat");
  assert.deepEqual(resampleWaveform(bytes, 0), []);
  // 3px bars with 2px gaps.
  assert.equal(barsFor(198), 40);
  assert.equal(barsFor(10), 8, "never fewer than eight");
  // The boundary bar fills partially as playback moves.
  assert.deepEqual(
    [0, 1, 2, 3].map((index) => barFill(index, 4, 0.6)),
    [1, 1, 0.3999999999999999, 0],
  );
  assert.equal(barFill(3, 4, 1), 1);
});

test("the live meter rises fast and falls slowly", () => {
  let level = 0;
  level = smoothLevel(level, 1);
  assert.equal(level, 0.6, "attack");
  const before = level;
  level = smoothLevel(level, 0);
  assert.ok(level < before && level > 0.5, "release is slower");
  assert.equal(smoothLevel(0.2, Number.NaN), 0.2 * 0.88);
});

test("sending as soon as recording stops is off unless the person turns it on", (t) => {
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => stored.set(key, value),
  });
  const events: string[] = [];
  vi.stubGlobal("window", { dispatchEvent: (event: Event) => events.push(event.type) });
  t.onTestFinished(() => {
    vi.unstubAllGlobals();
  });
  assert.equal(getVoiceSendOnStop(), false, "default: stage in the composer");
  setVoiceSendOnStop(true);
  assert.equal(getVoiceSendOnStop(), true);
  assert.equal(stored.get("caper:voice-send-on-stop"), "on");
  setVoiceSendOnStop(false);
  assert.equal(getVoiceSendOnStop(), false);
  assert.deepEqual(events, ["caper:voice-send-on-stop", "caper:voice-send-on-stop"], "open composers follow");
});

test("a staged voice message is a compact chip beside staged files, removable", () => {
  const html = renderToStaticMarkup(
    createElement(DraftAttachments, {
      drafts: [
        { key: "p", name: "shot.png", sourceSize: 10, progress: 1, localUrl: "blob:p", localKind: "image" },
        {
          key: "v",
          name: "Voice message",
          sourceSize: 2_000,
          progress: 0.5,
          localUrl: "blob:v",
          voice: { durationMs: 4_200, waveform: encodeWaveform(new Uint8Array(64).fill(128)) },
        },
      ],
      onRemove: () => {},
    }),
  );
  assert.match(html, /<li [^>]*data-voice="true"/);
  assert.match(html, /data-compact="true"/);
  assert.match(html, /aria-label="Play voice message"/);
  assert.match(html, />0:04</);
  assert.match(html, /aria-label="Remove voice message"/);
  assert.match(html, /aria-label="Remove shot\.png"/, "files keep their own tiles");
  assert.doesNotMatch(html, /aria-label="Spoiler attachment: Voice message"/, "no spoiler or edit for a voice chip");
});

test("removing a staged voice message cancels its upload", async () => {
  const calls: string[] = [];
  const transport: UploadTransport = {
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      calls.push(`${init?.method} ${String(input)}`);
      if (String(input) === "/api/assets")
        return Response.json({ id: "a9", upload: { method: "PUT", url: "https://r2.test/a9", headers: {} } });
      if (String(input).endsWith("/cancel")) return new Response(null, { status: 204 });
      return Response.json(voiceFile({ id: "a9" }));
    }) as typeof fetch,
    put: async (_url, _headers, _body, report) => report(1),
  };
  const upload = AttachmentUpload.voice(
    "chan1",
    { blob: new Blob([new Uint8Array(10)]), contentType: "audio/webm", durationMs: 1_500, waveform: "" },
    () => {},
    transport,
  );
  await upload.ensure();
  upload.release(true);
  assert.deepEqual(calls, ["POST /api/assets", "POST /api/assets/a9/complete", "POST /api/assets/a9/cancel"]);
  assert.equal(upload.source, undefined, "the local recording is dropped");
});

test("durations format as m:ss, rounding totals, flooring the timer and ceiling countdowns", () => {
  assert.equal(formatVoiceDuration(0), "0:00");
  assert.equal(formatVoiceDuration(4_200), "0:04");
  assert.equal(formatVoiceDuration(4_600), "0:05");
  assert.equal(formatVoiceDuration(4_600, "floor"), "0:04");
  assert.equal(formatVoiceDuration(100, "ceil"), "0:01", "a countdown never shows 0:00 before the end");
  assert.equal(formatVoiceDuration(65_000), "1:05");
  assert.equal(formatVoiceDuration(600_000), "10:00");
  assert.equal(formatVoiceDuration(3_723_000), "1:02:03");
  assert.equal(formatVoiceDuration(Number.NaN), "0:00");
  assert.equal(formatVoiceDuration(-5), "0:00");
  assert.equal(spokenDuration(65_000), "1 minute 5 seconds");
  assert.equal(spokenDuration(1_000), "1 second");
  assert.equal(spokenDuration(120_000), "2 minutes");
});

test("transcripts show only when pending or ready with text", () => {
  assert.deepEqual(transcriptView({ transcript: { status: "pending" } }), { state: "pending" });
  assert.deepEqual(transcriptView({ transcript: { status: "ready", text: " Yo what's going on " } }), {
    state: "ready",
    text: "Yo what's going on",
  });
  for (const transcript of [
    undefined,
    { status: "failed" },
    { status: "ready", text: "   " },
    { status: "ready" },
    { status: "queued", text: "future status" },
    "ready",
    null,
  ])
    assert.deepEqual(transcriptView({ transcript } as Pick<ChatAttachment, "transcript">), { state: "none" });
});

test("the transcript block: Transcribing… while pending, text when ready, nothing otherwise", () => {
  const render = (transcript: unknown) =>
    renderToStaticMarkup(createElement(VoiceTranscript, { attachment: { transcript } as ChatAttachment }));
  assert.match(render({ status: "pending" }), /Transcribing…/);
  const ready = render({ status: "ready", text: "Yo <b>what’s</b> going on" });
  assert.match(ready, /Yo &lt;b&gt;what’s&lt;\/b&gt; going on/, "plain, escaped text");
  assert.doesNotMatch(ready, /Transcribing/);
  for (const absent of [undefined, { status: "failed" }, { status: "ready", text: "" }, { status: "other" }])
    assert.equal(render(absent), "");
});

test("voice attachments validate, keep their fields and render the voice player; plain audio keeps its player", () => {
  const message = {
    content: {
      version: 1 as const,
      type: "text" as const,
      text: "",
      attachments: [
        voiceFile(),
        voiceFile({ id: "a2", voice: undefined, name: "song.mp3", contentType: "audio/mpeg" }),
      ],
    },
  };
  const [voice, song] = attachmentsOf(message);
  assert.ok(isChatAttachment(voice));
  assert.equal(voice.voice, true);
  assert.equal(voice.waveform, message.content.attachments[0].waveform);
  assert.deepEqual(voice.transcript, { status: "pending" });
  assert.ok(isVoiceAttachment(voice));
  assert.ok(!isVoiceAttachment(song));
  assert.ok(!isVoiceAttachment({ voice: true, kind: "file" }), "only audio plays as a voice message");

  const html = renderToStaticMarkup(createElement(MessageAttachments, { attachments: [voice, song] }));
  assert.match(html, /aria-label="Play voice message"/);
  assert.match(html, /role="slider"/);
  assert.equal(html.match(/<span style="height:/g)?.length, VOICE_BARS, "every bar before measuring");
  assert.match(html, />0:04</, "idle shows the total duration");
  assert.match(html, /Transcribing…/);
  assert.doesNotMatch(html, /Voice message\.m4a/, "the filename is not shown");
  assert.match(html, /<figcaption>song\.mp3<\/figcaption><audio controls/, "regular audio files keep today's UI");

  const flat = renderToStaticMarkup(
    createElement(MessageAttachments, { attachments: [voiceFile({ waveform: undefined, transcript: undefined })] }),
  );
  assert.match(flat, /data-flat="true"/, "a missing waveform draws a flat placeholder");
  assert.doesNotMatch(flat, /Transcribing|chat-voice-transcript/, "no transcript UI without a transcript");
});

test("excerpts call voice notes Voice message instead of their filename", () => {
  assert.equal(attachmentLabel(voiceFile()), "Voice message");
  assert.equal(attachmentLabel(voiceFile({ voice: undefined })), "Voice message.m4a");
  assert.equal(attachmentsExcerpt([voiceFile()]), "Voice message");
  assert.equal(attachmentsExcerpt([voiceFile(), voiceFile({ voice: false })]), "Attachment");
  assert.equal(attachmentsExcerpt([null]), "Attachment");
  assert.equal(attachmentsExcerpt(undefined), "Attachment");
});

test("a transcript arriving in message.attachments replaces the pending one, keeping every voice field", () => {
  const sent: ChatMessage = {
    id: "m1",
    channelId: "general",
    seq: "1",
    author: { id: "a", name: "A", isGuest: false },
    content: { version: 1, type: "text", text: "", attachments: [voiceFile()] },
    createdAt: "2026-10-20T00:00:00Z",
    clientMessageId: "c1",
  };
  const timeline = new ChatTimeline();
  timeline.reset([sent], "1");
  const ready = voiceFile({ transcript: { status: "ready", text: "Yo what's going on" } });
  const event: ChatAttachmentsEvent = {
    type: "message.attachments",
    schemaVersion: 1,
    channelId: "general",
    seq: "2",
    messageId: "m1",
    attachments: [ready],
  };
  assert.equal(timeline.applyEvent(event), "applied");
  const [file] = attachmentsOf(timeline.messages[0]);
  assert.notEqual(timeline.messages[0], sent, "a new message object, so its row re-renders");
  assert.deepEqual(file, ready);
  assert.deepEqual(transcriptView(file), { state: "ready", text: "Yo what's going on" });
  assert.equal(timeline.applyEvent({ ...event, attachments: [voiceFile()] }), "duplicate");
  assert.deepEqual(attachmentsOf(timeline.messages[0])[0].transcript, ready.transcript, "pending never returns");
});

test("recording prefers WebM Opus, then MP4, then Ogg Opus, and uploads the container type", () => {
  assert.equal(
    voiceRecordingType(() => true),
    "audio/webm;codecs=opus",
  );
  assert.equal(
    voiceRecordingType((type) => type !== "audio/webm;codecs=opus"),
    "audio/mp4",
  );
  assert.equal(
    voiceRecordingType((type) => type === "audio/ogg;codecs=opus"),
    "audio/ogg;codecs=opus",
  );
  assert.equal(
    voiceRecordingType(() => {
      throw new Error("unsupported");
    }),
    undefined,
  );
  assert.equal(voiceContentType("audio/webm;codecs=opus"), "audio/webm");
  assert.equal(voiceContentType("audio/mp4; codecs=mp4a.40.2"), "audio/mp4");
  assert.equal(voiceContentType(""), "audio/webm");
  assert.equal(voiceFilename("audio/mp4"), "voice-message.m4a");
  assert.equal(voiceFilename("audio/ogg"), "voice-message.ogg");
  assert.equal(voiceFilename("audio/webm"), "voice-message.webm");
});

test("a voice recording uploads as recorded with voice, duration and waveform, then sends the completed description", async () => {
  const reserved: Array<Record<string, unknown>> = [];
  const puts: Blob[] = [];
  const calls: string[] = [];
  const completed = voiceFile({ id: "asset1", size: 2_048 });
  const transport: UploadTransport = {
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      calls.push(`${init?.method} ${String(input)}`);
      if (String(input) === "/api/assets") {
        reserved.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
        return Response.json(
          { id: "asset1", upload: { method: "PUT", url: "https://r2.test/asset1", headers: {} } },
          { status: 201 },
        );
      }
      return Response.json(completed);
    }) as typeof fetch,
    put: async (_url, _headers, body, report) => {
      puts.push(body);
      report(1);
    },
  };
  const recording: VoiceRecording = {
    blob: new Blob([new Uint8Array(1_234)], { type: "audio/webm" }),
    contentType: "audio/webm",
    durationMs: 4_200,
    waveform: encodeWaveform(new Uint8Array(64).fill(9)),
  };
  const upload = AttachmentUpload.voice("chan1", recording, () => {}, transport);
  const attachment = await upload.ensure();
  assert.deepEqual(reserved, [
    {
      purpose: "attachment",
      channelId: "chan1",
      filename: "voice-message.webm",
      contentType: "audio/webm",
      byteSize: 1_234,
      sourceByteSize: 1_234,
      durationMs: 4_200,
      voice: true,
      waveform: recording.waveform,
      multipart: true,
    },
  ]);
  assert.equal(puts[0], recording.blob, "the recorded bytes go up untouched, never prepared or compressed");
  assert.deepEqual(calls, ["POST /api/assets", "POST /api/assets/asset1/complete"]);
  assert.deepEqual(attachment, completed, "the server's audio/mp4 description is what gets sent");
  assert.equal(attachment.contentType, "audio/mp4");
  assert.equal(voicePreparedFile({ ...recording, waveform: "" }).waveform, undefined, "no empty waveform field");
});

test("the profile card's DM composer offers the microphone and sends a voice message on its own", () => {
  assert.deepEqual(voiceCommand("asset1", "client-1"), {
    clientMessageId: "client-1",
    text: "",
    attachmentIds: ["asset1"],
    spoilerAttachmentIds: [],
  });
  assert.match(voiceCommand("asset1").clientMessageId, /^[0-9a-f-]{36}$/);
  const html = renderToStaticMarkup(
    createElement(ProfileComposer, {
      personId: "p1",
      username: "maya",
      inputRef: { current: null },
      onSent: () => {},
      onStateChange: () => {},
    }),
  );
  assert.match(html, /<form data-attach="true" data-voice="true"/);
  assert.match(
    html,
    /aria-label="Record voice message"[^>]*disabled=""/,
    "the microphone waits for upload settings, like the attach button",
  );
});
