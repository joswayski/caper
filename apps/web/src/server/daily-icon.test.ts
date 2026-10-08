import assert from "node:assert/strict";
import { test } from "vitest";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dailyIcon, dailyIconUrl } from "../components/daily-icon.ts";

test("wordmark exports preserve the original lettering and omit the static dot", () => {
  execFileSync(process.execPath, [
    new URL("../../../../scripts/generate-wordmark.mjs", import.meta.url).pathname,
    "--check",
  ]);
  const original = readFileSync(new URL("../../public/caper-wordmark.svg", import.meta.url), "utf8");
  const lettering = readFileSync(new URL("../../public/caper-wordmark-letters.svg", import.meta.url), "utf8");
  assert.equal((lettering.match(/<path\b/g) ?? []).length, 1);
  assert.equal(lettering.match(/<path[^>]+>/)?.[0], original.match(/<path[^>]+>/)?.[0]);
  assert.match(lettering, /viewBox="20 17 1042 276"/);
});

test("daily branding removes only the background from all 800 avatars without changing character paths", () => {
  assert.equal(dailyIconUrl(null), "/caper-face.svg?v=3");
  for (let id = 0; id < 800; id++) {
    assert.equal(dailyIconUrl(id), `/images/branding/v1/${id}.svg`);
    const original = readFileSync(new URL(`../../public/images/avatars/v3/${id}.svg`, import.meta.url), "utf8");
    const branding = readFileSync(new URL(`../../public${dailyIconUrl(id)}`, import.meta.url), "utf8");
    const paintedPaths = (svg: string) => [...svg.matchAll(/<path\b[^>]+fill="[^"]+"[^>]*\/>/g)].map(([path]) => path);
    assert.ok(paintedPaths(original).length > 1);
    assert.deepEqual(paintedPaths(branding), paintedPaths(original).slice(1), `Character changed for ID ${id}`);
    assert.match(branding, /viewBox="0 0 256 256"/);
    assert.doesNotMatch(branding, /<clipPath|clip-path|<image|data:/i);
  }
});

test("native branding exports preserve the web character geometry, palette and translations", () => {
  for (let id = 0; id < 800; id++) {
    const branding = readFileSync(new URL(`../../public/images/branding/v1/${id}.svg`, import.meta.url), "utf8");
    const apple = readFileSync(
      new URL(
        `../../../native/apple/Sources/CaperCore/CaperAvatars.xcassets/caper-branding-${id}.imageset/avatar.svg`,
        import.meta.url,
      ),
      "utf8",
    );
    assert.equal(apple, branding, `Apple branding drift for ${id}`);
    const android = readFileSync(
      new URL(`../../../native/android/app/src/main/res/drawable/caper_branding_${id}.xml`, import.meta.url),
      "utf8",
    );
    const paths = [
      ...branding.matchAll(/<path d="([^"]+)" fill="([^"]+)" transform="translate\(([^,]+),([^)]+)\)"\/>/g),
    ].map(([, d, fill, x, y]) => [x, y, d, fill]);
    const vectors = [
      ...android.matchAll(
        /<group android:translateX="([^"]+)" android:translateY="([^"]+)"><path android:pathData="([^"]+)" android:fillColor="([^"]+)"\/><\/group>/g,
      ),
    ].map(([, x, y, d, fill]) => [x, y, d, fill]);
    assert.ok(paths.length > 0);
    assert.deepEqual(vectors, paths, `Android branding drift for ${id}`);
    assert.doesNotMatch(android, /<clip-path/);
  }
});

test("daily icon remains stable until the UTC boundary", () => {
  const saved = { day: 0, index: 143 };
  assert.deepEqual(
    dailyIcon(saved, 86_399_999, () => {
      throw Error("must not draw");
    }),
    saved,
  );
  assert.deepEqual(
    dailyIcon(saved, 86_400_000, () => 0),
    { day: 1, index: 0 },
  );
});

test("daily icon selects all 800 avatars and excludes the previous one without losing the last index", () => {
  assert.deepEqual(
    new Set(Array.from({ length: 800 }, (_, i) => dailyIcon(null, 0, () => (i + 0.5) / 800).index)),
    new Set(Array.from({ length: 800 }, (_, i) => i)),
  );
  for (const previous of [0, 143, 798, 799]) {
    const selected = new Set(
      Array.from(
        { length: 799 },
        (_, i) => dailyIcon({ day: 0, index: previous }, 86_400_000, () => (i + 0.5) / 799).index,
      ),
    );
    assert.equal(selected.size, 799);
    assert.equal(selected.has(previous), false);
    assert.equal(selected.has(799), previous !== 799);
  }
});

test("invalid saved IDs do not pin an invalid icon on the current day", () => {
  for (const index of [-1, 800, 1.5, NaN])
    assert.deepEqual(
      dailyIcon({ day: 0, index }, 0, () => 0),
      { day: 0, index: 0 },
    );
});
