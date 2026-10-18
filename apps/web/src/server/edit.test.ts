import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import {
  clampCrop,
  cropPixels,
  isEdited,
  NO_EDIT,
  paintEdit,
  rotateLeft,
  type AttachmentEdit,
  type PixelRect,
} from "../chat/edit.ts";

// Every client runs these same cases; see shared/messages/edit-cases.json.
const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/edit-cases.json", import.meta.url), "utf8"),
) as {
  rotateLeft: { note: string; before: AttachmentEdit; after: AttachmentEdit }[];
  cropPixels: {
    note: string;
    width: number;
    height: number;
    rotation: AttachmentEdit["rotation"];
    crop: AttachmentEdit["crop"];
    even: boolean;
    expected: PixelRect;
  }[];
  edited: { note: string; edit: AttachmentEdit; edited: boolean }[];
};

/** Deep equality with fractions compared to 1e-9. */
function close(actual: unknown, expected: unknown, note: string) {
  if (typeof expected === "number") {
    assert.equal(typeof actual, "number", note);
    assert.ok(Math.abs((actual as number) - expected) < 1e-9, `${note}: ${String(actual)} ≠ ${expected}`);
  } else if (Array.isArray(expected)) {
    assert.ok(Array.isArray(actual) && actual.length === expected.length, note);
    expected.forEach((value, index) => close((actual as unknown[])[index], value, note));
  } else if (expected && typeof expected === "object") {
    assert.deepEqual(Object.keys(actual as object).sort(), Object.keys(expected).sort(), note);
    for (const [key, value] of Object.entries(expected)) close((actual as Record<string, unknown>)[key], value, note);
  } else assert.equal(actual, expected, note);
}

test("shared rotate-left cases match", () => {
  assert.ok(shared.rotateLeft.length >= 4);
  for (const { note, before, after } of shared.rotateLeft) close(rotateLeft(before), after, note);
});

test("four left turns restore the edit", () => {
  for (const { before, note } of shared.rotateLeft)
    close(rotateLeft(rotateLeft(rotateLeft(rotateLeft(before)))), before, note);
});

test("shared crop pixel cases match", () => {
  assert.ok(shared.cropPixels.length >= 7);
  for (const { note, width, height, rotation, crop, even, expected } of shared.cropPixels)
    assert.deepEqual(cropPixels({ rotation, crop, marks: [] }, width, height, even), expected, note);
});

test("shared edited cases match", () => {
  for (const { note, edit, edited } of shared.edited) assert.equal(isEdited(edit), edited, note);
  assert.equal(isEdited(undefined), false);
});

test("crops stay inside the image with a minimum size", () => {
  assert.deepEqual(clampCrop({ x: 0.9, y: -0.2, width: 0.5, height: 0.001 }), {
    x: 0.5,
    y: 0,
    width: 0.5,
    height: 0.02,
  });
  assert.deepEqual(clampCrop({ x: 0, y: 0, width: 3, height: 1 }), { x: 0, y: 0, width: 1, height: 1 });
});

test("painting turns the original, then crops and scales it", () => {
  const calls: string[] = [];
  const context = new Proxy(
    {},
    {
      get:
        (_, name: string) =>
        (...args: unknown[]) =>
          calls.push(
            `${name}(${args.map((value) => (typeof value === "number" ? Math.round(value * 100) / 100 : "·")).join(",")})`,
          ),
      set: () => true,
    },
  ) as unknown as CanvasRenderingContext2D;
  const edit: AttachmentEdit = {
    ...NO_EDIT,
    rotation: 90,
    marks: [{ kind: "stroke", color: "#000000", width: 0.01, points: [0.5, 0.5] }],
  };
  // A 400 × 300 original turned clockwise is 300 × 400; a dot of 1% of 400 px.
  paintEdit(context, {} as CanvasImageSource, 400, 300, edit, { left: 10, top: 20, width: 100, height: 100 }, 2);
  assert.deepEqual(calls, [
    "save()",
    "scale(2,2)",
    "translate(-10,-20)",
    "save()",
    "translate(300,0)",
    "rotate(1.57)",
    "drawImage(·,0,0,400,300)",
    "restore()",
    "beginPath()",
    "arc(150,200,2,0,6.28)",
    "fill()",
    "restore()",
  ]);
});
