import { describe, expect, it } from "vitest";
import { displayP3ToSrgb, heicIsDisplayP3, isHeicFile } from "./heic.ts";

const bytes = (...parts: (string | ArrayLike<number>)[]) =>
  new Uint8Array(
    parts.flatMap((part) => (typeof part === "string" ? [...part].map((c) => c.charCodeAt(0)) : Array.from(part))),
  );
const u32 = (value: number) => [value >>> 24, (value >> 16) & 255, (value >> 8) & 255, value & 255];
const ftyp = (major: string, ...compatible: string[]) =>
  bytes(u32(16 + 4 * compatible.length), "ftyp", major, [0, 0, 0, 0], ...compatible, u32(8), "avif");

describe("HEIC", () => {
  it("tells HEIC stills from AVIF, sequences and other files", () => {
    expect(isHeicFile(ftyp("heic", "mif1", "heic"))).toBe(true);
    expect(isHeicFile(ftyp("mif1", "mif1", "heic"))).toBe(true);
    // The box after ftyp must not be read as a compatible brand.
    expect(isHeicFile(ftyp("mif1", "mif1", "miaf"))).toBe(true);
    expect(isHeicFile(ftyp("mif1", "mif1", "avif"))).toBe(false);
    for (const major of ["avif", "avis", "msf1", "hevc", "isom"]) expect(isHeicFile(ftyp(major, "mif1"))).toBe(false);
    expect(isHeicFile(bytes([0xff, 0xd8, 0xff, 0xe0]))).toBe(false);
  });

  it("reads Display P3 from an nclx colour box or an ICC profile", () => {
    const nclx = (primaries: number) => bytes(ftyp("heic"), u32(19), "colrnclx", [0, primaries, 0, 13, 0, 1, 0x80]);
    expect(heicIsDisplayP3(nclx(12))).toBe(true);
    expect(heicIsDisplayP3(nclx(1))).toBe(false);
    const profile = (name: number[]) =>
      bytes(ftyp("heic"), u32(12 + 40 + name.length), "colrprof", Array(40).fill(0), name);
    const utf16 = [..."Display P3"].flatMap((c) => [0, c.charCodeAt(0)]);
    expect(heicIsDisplayP3(profile(utf16))).toBe(true);
    expect(heicIsDisplayP3(profile([..."sRGB IEC61966-2.1"].map((c) => c.charCodeAt(0))))).toBe(false);
    expect(heicIsDisplayP3(ftyp("heic"))).toBe(false);
  });

  it("converts Display P3 to sRGB", () => {
    // Expected values from the CSS Color 4 matrices through XYZ; alpha stays.
    const rgba = new Uint8ClampedArray([128, 128, 128, 255, 200, 100, 50, 255, 30, 180, 90, 9]);
    displayP3ToSrgb(rgba);
    const expected = [128, 128, 128, 255, 215, 93, 31, 255, 0, 183, 78, 9];
    rgba.forEach((value, i) => expect(Math.abs(value - expected[i])).toBeLessThanOrEqual(1));
  });
});
