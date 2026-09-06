// Re-extract the pinned RNNoise binary. Not needed for normal builds.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const directory = new URL("../apps/web/public/audio/rnnoise-v1/", import.meta.url);
const temporary = await mkdtemp(join(tmpdir(), "caper-rnnoise-"));
async function download(url) {
  const response = await fetch(url, { signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error(`Download failed: ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}
function verify(bytes, algorithm, encoding, expected) {
  if (createHash(algorithm).update(bytes).digest(encoding) !== expected) throw new Error("Asset checksum mismatch");
}
try {
  const archive = await download("https://registry.npmjs.org/@shiguredo/rnnoise-wasm/-/rnnoise-wasm-2025.1.5.tgz");
  verify(archive, "sha512", "base64", "9YJxzzHftlW936E5z9aEKK2CrPfUgc3HgZ2keyQ5e2Pb5bx+6tykYQ2Gyudvxd7Sxmlnar7SemDMFfx0OuVuGA==");
  await writeFile(join(temporary, "package.tgz"), archive);
  execFileSync("tar", ["-xzf", join(temporary, "package.tgz"), "-C", temporary, "package/dist/rnnoise.js", "package/LICENSE"]);
  const source = await readFile(join(temporary, "package/dist/rnnoise.js"), "utf8");
  const payload = source.match(/["'](AGFzbQ[A-Za-z0-9+/=]+)["']/)?.[1];
  if (!payload) throw new Error("Embedded WASM not found");
  const wasm = Buffer.from(payload, "base64");
  verify(wasm, "sha256", "hex", "b3b67c9eae8f0791aad468c708659e0850bb37b0fb9c8a8666f2d7b0b6869bc4");
  const license = await download("https://raw.githubusercontent.com/xiph/rnnoise/70f1d256acd4b34a572f999a05c87bf00b67730d/COPYING");
  verify(license, "sha256", "hex", "45d37ca1cdb278c088e1aa85e0e65ca3a534ed86a28dcc96ca16810248a61d35");
  await mkdir(directory, { recursive: true });
  await writeFile(new URL("rnnoise.wasm", directory), wasm);
  await writeFile(new URL("COPYING", directory), license);
  await writeFile(new URL("LICENSE-APACHE", directory), await readFile(join(temporary, "package/LICENSE")));
  console.log(`Verified and extracted RNNoise (${wasm.length} bytes)`);
} finally {
  await rm(temporary, { recursive: true, force: true });
}
