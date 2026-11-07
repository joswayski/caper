// Writes cases.json from inputs/ with the web cleaner, after checking each
// cleaned output: no "secret" bytes or GPS left, and it still decodes. Run by
// generate.sh: node --experimental-transform-types cases.ts <metadata.ts> <dir>
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [cleaner, dir] = process.argv.slice(2);
const { cleanMetadata, MetadataRefused } = await import(cleaner);

/** Declared type per input; a second entry checks bytes win over the label. */
const CASES: [string, string][] = [
  ["photo.jpg", "image/jpeg"],
  ["photo.jpg", "application/octet-stream"],
  ["no-jfif.jpg", "image/jpeg"],
  ["trailer.jpg", "image/jpeg"],
  ["progressive.jpg", "image/jpeg"],
  ["photo.png", "image/png"],
  ["photo.gif", "image/gif"],
  ["photo.webp", "image/webp"],
  ["plain.webp", "image/webp"],
  ["photo.heic", "image/heic"],
  ["photo.avif", "image/avif"],
  ["photo.tif", "image/tiff"],
  ["big-endian.tif", "image/tiff"],
  ["maker-canon.tif", "image/x-canon-cr2"],
  ["maker-nikon.tif", "image/x-nikon-nef"],
  ["maker-olympus.tif", "image/x-olympus-orf"],
  ["maker-panasonic.tif", "image/x-panasonic-rw2"],
  ["maker-pentax.tif", "image/x-pentax-pef"],
  ["photo.psd", "image/vnd.adobe.photoshop"],
  ["photo.jp2", "image/jp2"],
  ["tone.mp3", "audio/mpeg"],
  ["tone.m4a", "audio/mp4"],
  ["cover.m4a", "audio/x-m4a"],
  ["tone.aac", "audio/aac"],
  ["tone.flac", "audio/flac"],
  ["tone.ogg", "audio/ogg"],
  ["tone.opus", "audio/ogg"],
  ["flac.oga", "audio/ogg"],
  ["tone.spx", "audio/ogg"],
  ["tone.wav", "audio/wav"],
  ["tone.aiff", "audio/aiff"],
  ["clip.mp4", "video/mp4"],
  ["clip.mov", "video/quicktime"],
  ["subtitled.mp4", "video/mp4"],
  ["clip.webm", "video/webm"],
  ["subtitled.webm", "video/webm"],
  ["clip.ogv", "video/ogg"],
  ["clip.avi", "video/x-msvideo"],
  ["bare.jxl", "image/jxl"],
  ["document.pdf", "application/pdf"],
  ["picture.bmp", "image/bmp"],
  ["picture.svg", "image/svg+xml"],
  ["canon.cr3", "image/x-canon-cr3"],
  ["truncated.png", "image/png"],
];
const STILLS = /\.(jpe?g|png|gif|webp|heic|avif|tif|psd|jp2)$/;

const scratch = mkdtempSync(join(tmpdir(), "metadata-cases-"));
const cases = [];
const failures: string[] = [];
for (const [file, contentType] of CASES) {
  const input = readFileSync(join(dir, "inputs", file));
  let expected: string;
  try {
    const output = Buffer.from(await (await cleanMetadata(new Blob([input]), contentType)).arrayBuffer());
    expected = createHash("sha256").update(output).digest("hex");
    const path = join(scratch, file);
    writeFileSync(path, output);
    if (/secret/i.test(output.toString("latin1")) && !file.endsWith(".pdf"))
      failures.push(`${file}: "secret" bytes remain`);
    if (!/\.(pdf|jxl)$/.test(file)) {
      const flags = ["-a", "-G1", "-s", "-n", "-gps:all", "-location*", "-*serial*", "-*owner*", "-artist"];
      const tags = execFileSync("exiftool", [...flags, path])
        .toString()
        .split("\n")
        // Blanked maker-note values are still listed. Pentax keeps an internal
        // serial inside a binary block, as ExifTool on the server did.
        .filter(
          (line) =>
            line && !/:\s*0*\s*$/.test(line) && !/^\[Pentax\]\s+InternalSerialNumber|SerialNumberFormat/.test(line),
        );
      if (tags.length) failures.push(`${file}: ${tags.join("; ")}`);
    }
    try {
      if (STILLS.test(file)) execFileSync("magick", [path, "-format", "%wx%h", "info:"], { stdio: "pipe" });
      else if (!/\.(pdf|jxl)$/.test(file))
        execFileSync("ffmpeg", ["-v", "error", "-xerror", "-i", path, "-f", "null", "-"], { stdio: "pipe" });
    } catch (error) {
      failures.push(`${file}: does not decode: ${(error as { stderr?: Buffer }).stderr}`);
    }
  } catch (error) {
    if (!(error instanceof MetadataRefused)) throw error;
    expected = `refused:${error.reason}`;
  }
  cases.push({ file, contentType, expected });
}
rmSync(scratch, { recursive: true });
if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}
writeFileSync(join(dir, "cases.json"), `${JSON.stringify(cases, null, 2)}\n`);
console.log(`${cases.length} cases`);
