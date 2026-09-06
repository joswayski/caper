import { createHash } from "node:crypto";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";

const revision = "dd6818d00f50c836fed43a6243ebe49116de5964";
const destination = new URL("../apps/web/public/audio/dpdfnet2-v1/", import.meta.url);
await mkdir(destination, { recursive: true });
const runtime = JSON.parse(await readFile(new URL("../node_modules/onnxruntime-web/package.json", import.meta.url), "utf8"));
if (runtime.version !== "1.23.2") throw new Error("Unexpected ONNX Runtime version");
const response = await fetch(`https://huggingface.co/Ceva-IP/DPDFNet/resolve/${revision}/onnx/dpdfnet2_48khz_hr.onnx?download=true`, { signal: AbortSignal.timeout(60_000) });
if (!response.ok) throw new Error(`model download failed: ${response.status}`);
const model = Buffer.from(await response.arrayBuffer());
if (createHash("sha256").update(model).digest("hex") !== "7f0575a5cec0ba4ffd8f8bd657e06d007e4ccdd955d76faab922b9d3291dc14b") throw new Error("model checksum mismatch");
await writeFile(new URL("dpdfnet2_48khz_hr.onnx", destination), model);
for (const name of ["ort.wasm.bundle.min.mjs", "ort-wasm-simd-threaded.mjs", "ort-wasm-simd-threaded.wasm"]) {
  await cp(new URL(`../node_modules/onnxruntime-web/dist/${name}`, import.meta.url), new URL(name, destination));
}
for (const [name, url, hash] of [
  ["LICENSE-APACHE-2.0", "https://raw.githubusercontent.com/ceva-ip/DPDFNet/1333776d470f01ecf4a533f098f4e8aeb3d00b89/LICENSE", "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4"],
  ["ORT-LICENSE", "https://raw.githubusercontent.com/microsoft/onnxruntime/v1.23.2/LICENSE", "2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c"],
  ["ORT-ThirdPartyNotices.txt", "https://raw.githubusercontent.com/microsoft/onnxruntime/v1.23.2/ThirdPartyNotices.txt", "e9e90971a8e75a9a8ac0c6412e29c1202d079998389915aa485f46c816c3b4cc"],
]) {
  const response = await fetch(url, { signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error(`License download failed: ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (createHash("sha256").update(bytes).digest("hex") !== hash) throw new Error("License checksum mismatch");
  await writeFile(new URL(name, destination), bytes);
}
const exporter = `import json,onnx,sys\nm=onnx.load(sys.argv[1]);d={x.key:x.value for x in m.metadata_props};o={'stateSize':int(d['state_size']),'erbNormStateSize':int(d['erb_norm_state_size']),'erbNormInit':[float(x) for x in d['erb_norm_init'].split(',')],'specNormInit':[float(x) for x in d['spec_norm_init'].split(',')]};open(sys.argv[2],'w').write(json.dumps(o,separators=(',',':'))+'\\n')`;
const result = spawnSync("python", ["-c", exporter, new URL("dpdfnet2_48khz_hr.onnx", destination).pathname, new URL("metadata.json", destination).pathname], { stdio: "inherit" });
if (result.status !== 0) throw new Error("metadata export failed (install Python package `onnx`)");
