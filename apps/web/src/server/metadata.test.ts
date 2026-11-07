import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { cleanMetadata, MetadataRefused } from "../chat/metadata.ts";

const fixtures = new URL("../../../../shared/media/metadata/", import.meta.url);
const cases = JSON.parse(readFileSync(new URL("cases.json", fixtures), "utf8")) as {
  file: string;
  contentType: string;
  expected: string;
}[];

async function outcome(blob: Blob, contentType: string) {
  try {
    const cleaned = await cleanMetadata(blob, contentType);
    return createHash("sha256")
      .update(new Uint8Array(await cleaned.arrayBuffer()))
      .digest("hex");
  } catch (error) {
    if (error instanceof MetadataRefused) return `refused:${error.reason}`;
    throw error;
  }
}

// Every client must produce these exact bytes (shared/media/metadata/cases.json).
test.each(cases)("$file as $contentType matches the shared fixture", async ({ file, contentType, expected }) => {
  const input = readFileSync(new URL(`inputs/${file}`, fixtures));
  assert.equal(await outcome(new Blob([input]), contentType), expected);
});

test("a large video is cleaned from its box headers, without reading the samples", async () => {
  const input = readFileSync(new URL("inputs/clip.mov", fixtures));
  // clip.mov keeps `moov` after `mdat`; grow the samples to 200 MB.
  const mdat = input.indexOf("mdat") - 4;
  const size = input.readUInt32BE(mdat);
  const padding = 200 * 1024 * 1024;
  const grown = Buffer.from(input);
  grown.writeUInt32BE(size + padding, mdat);
  const blob = new Blob([grown.subarray(0, mdat + size), new Uint8Array(padding), grown.subarray(mdat + size)]);
  let read = 0;
  const slice = blob.slice.bind(blob);
  blob.slice = (start?: number, end?: number) => {
    const part = slice(start, end);
    const arrayBuffer = part.arrayBuffer.bind(part);
    part.arrayBuffer = async () => {
      const bytes = await arrayBuffer();
      read += bytes.byteLength;
      return bytes;
    };
    return part;
  };
  const cleaned = await cleanMetadata(blob, "video/quicktime");
  assert.equal(cleaned.size, blob.size);
  assert.ok(read < 4 * 1024 * 1024, `read ${read} bytes`);
  const tail = new Uint8Array(await cleaned.slice(mdat + size + padding).arrayBuffer());
  assert.ok(!Buffer.from(tail).includes("Secret"));
});
