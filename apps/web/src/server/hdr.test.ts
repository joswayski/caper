import assert from "node:assert/strict";
import { test } from "vitest";
import {
  BT2020_TO_BT709,
  HDR_PEAK_NITS,
  SDR_REFERENCE_NITS,
  bt709Oetf,
  hable,
  hlgOotf,
  hlgToScene,
  pqToNits,
  supportedHdrFrameFormat,
  toneMapPixel,
} from "../chat/hdr.ts";

const near = (actual: number, expected: number, tolerance: number, message?: string) =>
  assert.ok(Math.abs(actual - expected) <= tolerance, `${message ?? ""} ${actual} ≉ ${expected}`);

test("PQ (SMPTE ST 2084) decodes to absolute nits", () => {
  near(pqToNits(0), 0, 1e-9);
  near(pqToNits(1), 10_000, 1e-6);
  near(pqToNits(0.5807), 203, 1, "BT.2408 reference white");
  near(pqToNits(0.7518), 1000, 2);
});

test("HLG (ARIB STD-B67) inverse OETF and BT.2100 OOTF", () => {
  near(hlgToScene(0.5), 1 / 12, 1e-6, "the two segments meet");
  near(hlgToScene(1), 1, 1e-6);
  const white = hlgToScene(0.75);
  near(hlgOotf([white, white, white])[0], 203, 3, "75% HLG is 203-nit reference white on a 1000-nit display");
  near(hlgOotf([1, 1, 1])[0], HDR_PEAK_NITS, 1e-6);
  const [r, g, b] = hlgOotf([0.2, 0.1, 0.05]);
  near(r / g, 2, 1e-9, "the OOTF scales by luminance, keeping hue");
  near(g / b, 2, 1e-9);
});

test("Hable reaches 1 at the peak, is monotonic, and BT.709 OETF matches the spec", () => {
  const peak = HDR_PEAK_NITS / SDR_REFERENCE_NITS;
  near(hable(peak), 1, 1e-9);
  assert.equal(hable(peak * 4), 1);
  let last = -1;
  for (let x = 0; x <= peak; x += 0.25) {
    assert.ok(hable(x) > last);
    last = hable(x);
  }
  near(bt709Oetf(0.01), 0.045, 1e-9);
  near(bt709Oetf(1), 1, 1e-9);
  // Each row of the primaries matrix sums to 1: white stays white.
  for (let row = 0; row < 3; row++)
    near(BT2020_TO_BT709[row * 3] + BT2020_TO_BT709[row * 3 + 1] + BT2020_TO_BT709[row * 3 + 2], 1, 2e-3);
});

test("tone mapped pixels: neutral stays neutral, black stays black, reference white is bright, primaries stay saturated", () => {
  for (const transfer of ["pq", "hlg"] as const) {
    assert.deepEqual(toneMapPixel(64, 512, 512, transfer), [16, 128, 128], `${transfer} black`);
    const signal = transfer === "pq" ? 0.5807 : 0.75; // 203 nits
    const [y, cb, cr] = toneMapPixel(64 + 876 * signal, 512, 512, transfer);
    assert.ok(y > 160 && y < 200, `${transfer} reference white ${y}`);
    assert.deepEqual([cb, cr], [128, 128], `${transfer} grey has no chroma`);
  }
  // HDR10 red (BT.709 red inside BT.2020, 203 nits): saturated red, not pink.
  const [, cb, cr] = toneMapPixel(391, 439, 608, "pq");
  assert.ok(cr > 200 && cb < 120, `red ${cb} ${cr}`);
});

test("only planar YUV frames the shader understands take the GPU path", () => {
  assert.ok(supportedHdrFrameFormat("I420P10"));
  assert.ok(supportedHdrFrameFormat("NV12"));
  assert.ok(!supportedHdrFrameFormat("RGBA"));
  assert.ok(!supportedHdrFrameFormat(null));
});
