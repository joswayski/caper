// HDR (BT.2100 PQ / HLG) to SDR BT.709 tone mapping on the GPU, so HDR phone
// video can be transcoded instead of uploaded as an original. Browsers do not
// tone map consistently when drawing an HDR VideoFrame, so this reads the
// decoded 10-bit planes itself and does every step explicitly:
//
//   Y'CbCr (BT.2020 NCL) → R'G'B' → linear light (inverse PQ, or HLG inverse
//   OETF + OOTF) in nits → ÷ SDR reference white → BT.2020 → BT.709 primaries
//   → Hable tone curve on max(R,G,B) → (downscale in linear light, mipmapped)
//   → BT.709 OETF → BT.709 limited-range 8-bit I420.
//
// The shader writes the I420 planes directly (four samples per RGBA8 texel),
// so the encoder receives frames already tagged BT.709 and never applies its
// own RGB → YUV conversion. The scalar functions below mirror the shader and
// are unit tested in Node.

export type HdrTransfer = "pq" | "hlg";

/** Nits that map to SDR "1.0" before the tone curve. 100 nits keeps HLG/PQ
 * diffuse white (203 nits, BT.2408) close to SDR brightness after Hable. */
export const SDR_REFERENCE_NITS = 100;
/** Assumed mastering peak: HLG's nominal display and the common HDR10 grade. */
export const HDR_PEAK_NITS = 1000;
const HLG_GAMMA = 1.2; // BT.2100 system gamma at a 1000-nit nominal peak

const PQ = { m1: 2610 / 16384, m2: 2523 / 4096 * 128, c1: 3424 / 4096, c2: 2413 / 4096 * 32, c3: 2392 / 4096 * 32 };
const HLG = { a: 0.17883277, b: 0.28466892, c: 0.55991073 };
const HABLE = { A: 0.15, B: 0.5, C: 0.1, D: 0.2, E: 0.02, F: 0.3 };

/** SMPTE ST 2084 EOTF: non-linear signal [0, 1] to display nits. */
export function pqToNits(signal: number) {
  const p = Math.max(signal, 0) ** (1 / PQ.m2);
  return 10_000 * (Math.max(p - PQ.c1, 0) / (PQ.c2 - PQ.c3 * p)) ** (1 / PQ.m1);
}

/** ARIB STD-B67 inverse OETF: signal [0, 1] to normalized scene light [0, 1]. */
export function hlgToScene(signal: number) {
  const e = Math.max(signal, 0);
  return e <= 0.5 ? e * e / 3 : (Math.exp((e - HLG.c) / HLG.a) + HLG.b) / 12;
}

/** BT.2100 HLG OOTF: scene-light RGB (BT.2020) to display nits. */
export function hlgOotf(rgb: [number, number, number], peakNits = HDR_PEAK_NITS): [number, number, number] {
  const ys = 0.2627 * rgb[0] + 0.678 * rgb[1] + 0.0593 * rgb[2];
  const gain = peakNits * Math.max(ys, 1e-6) ** (HLG_GAMMA - 1);
  return [rgb[0] * gain, rgb[1] * gain, rgb[2] * gain];
}

function hableCurve(x: number) {
  const { A, B, C, D, E, F } = HABLE;
  return (x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F) - E / F;
}

/** Hable (Uncharted 2) filmic curve, normalized so `peak` maps to 1. */
export function hable(x: number, peak = HDR_PEAK_NITS / SDR_REFERENCE_NITS) {
  return Math.min(1, hableCurve(Math.max(x, 0)) / hableCurve(peak));
}

/** BT.709 OETF: linear [0, 1] to signal. */
export function bt709Oetf(linear: number) {
  const l = Math.min(Math.max(linear, 0), 1);
  return l < 0.018 ? 4.5 * l : 1.099 * l ** 0.45 - 0.099;
}

/** Linear BT.2020 RGB to linear BT.709 RGB (ITU-R BT.2087). Row-major. */
export const BT2020_TO_BT709 = [
  1.6605, -0.5876, -0.0728,
  -0.1246, 1.1329, -0.0083,
  -0.0182, -0.1006, 1.1187,
] as const;

/** Full CPU reference of the shader for one limited-range 10-bit BT.2020 pixel;
 * returns limited-range 8-bit BT.709 Y'CbCr. */
export function toneMapPixel(y: number, cb: number, cr: number, transfer: HdrTransfer, bitDepth = 10, referenceNits = SDR_REFERENCE_NITS): [number, number, number] {
  const scale = 2 ** (bitDepth - 8);
  const Y = (y - 16 * scale) / (219 * scale), U = (cb - 128 * scale) / (224 * scale), V = (cr - 128 * scale) / (224 * scale);
  const signal = [Y + 1.4746 * V, Y - 0.16455 * U - 0.57135 * V, Y + 1.8814 * U].map((v) => Math.min(Math.max(v, 0), 1)) as [number, number, number];
  const nits = transfer === "pq" ? signal.map(pqToNits) as [number, number, number] : hlgOotf(signal.map(hlgToScene) as [number, number, number]);
  const m = BT2020_TO_BT709;
  const rel = nits.map((n) => n / referenceNits);
  const rgb = [0, 1, 2].map((row) => Math.max(0, m[row * 3] * rel[0] + m[row * 3 + 1] * rel[1] + m[row * 3 + 2] * rel[2]));
  const peak = Math.max(...rgb);
  const gain = peak > 0 ? hable(peak, HDR_PEAK_NITS / referenceNits) / peak : 0;
  const [r, g, b] = rgb.map((v) => bt709Oetf(v * gain));
  const luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return [Math.round(16 + 219 * luma), Math.round(128 + 224 * (b - luma) / 1.8556), Math.round(128 + 224 * (r - luma) / 1.5748)];
}

// ---- WebGL2 ---------------------------------------------------------------

/** Pixel formats whose planes the mapper can read. */
const FORMATS: Record<string, { depth: number; chroma: [number, number]; nv12?: boolean }> = {
  I420: { depth: 8, chroma: [1, 1] }, I420P10: { depth: 10, chroma: [1, 1] }, I420P12: { depth: 12, chroma: [1, 1] },
  I422: { depth: 8, chroma: [1, 0] }, I422P10: { depth: 10, chroma: [1, 0] }, I422P12: { depth: 12, chroma: [1, 0] },
  I444: { depth: 8, chroma: [0, 0] }, I444P10: { depth: 10, chroma: [0, 0] }, I444P12: { depth: 12, chroma: [0, 0] },
  NV12: { depth: 8, chroma: [1, 1], nv12: true },
};

export const supportedHdrFrameFormat = (format: string | null | undefined) => !!format && format in FORMATS;

const VERTEX = `#version 300 es
in vec2 position;
void main() { gl_Position = vec4(position, 0.0, 1.0); }`;

// Pass 1: decode, linearize, tone map. Output linear BT.709 at source size.
const DECODE = `#version 300 es
precision highp float; precision highp int; precision highp usampler2D;
uniform usampler2D planeY, planeU, planeV;
uniform bool nv12;
uniform ivec2 chromaShift, origin;
uniform vec4 range; // yOffset, yScale, cOffset, cScale (code values)
uniform mat3 yuvToRgb, primaries; // column-major
uniform bool hlg, convertPrimaries;
uniform float lumaR, lumaG, lumaB, referenceNits, peakNits;
out vec4 color;

float hableCurve(float x) {
  const float A = 0.15, B = 0.5, C = 0.1, D = 0.2, E = 0.02, F = 0.3;
  return (x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F) - E / F;
}
vec2 chromaAt(ivec2 p) {
  ivec2 size = textureSize(planeU, 0);
  p = clamp(p, ivec2(0), size - 1);
  if (nv12) return vec2(texelFetch(planeU, p, 0).rg);
  return vec2(float(texelFetch(planeU, p, 0).r), float(texelFetch(planeV, p, 0).r));
}
void main() {
  ivec2 p = origin + ivec2(gl_FragCoord.xy);
  float y = float(texelFetch(planeY, p, 0).r);
  // Chroma is co-sited horizontally with even luma and centred vertically
  // between luma rows (H.264/HEVC/VP9 default), sampled bilinearly.
  vec2 c = vec2(chromaShift.x == 1 ? float(p.x) * 0.5 : float(p.x), chromaShift.y == 1 ? (float(p.y) - 0.5) * 0.5 : float(p.y));
  vec2 c0 = floor(c);
  vec2 f = c - c0;
  ivec2 i0 = ivec2(c0);
  vec2 uv = mix(mix(chromaAt(i0), chromaAt(i0 + ivec2(1, 0)), f.x), mix(chromaAt(i0 + ivec2(0, 1)), chromaAt(i0 + ivec2(1, 1)), f.x), f.y);
  vec3 yuv = vec3((y - range.x) / range.y, (uv - range.z) / range.w);
  vec3 signal = clamp(yuvToRgb * yuv, 0.0, 1.0);
  vec3 nits;
  if (hlg) {
    vec3 low = signal * signal / 3.0;
    vec3 high = (exp((signal - 0.55991073) / 0.17883277) + 0.28466892) / 12.0;
    vec3 scene = mix(low, high, step(0.5, signal));
    float ys = dot(scene, vec3(lumaR, lumaG, lumaB));
    nits = scene * peakNits * pow(max(ys, 1e-6), 0.2);
  } else {
    vec3 e = pow(signal, vec3(1.0 / 78.84375));
    nits = 10000.0 * pow(max(e - 0.8359375, 0.0) / (18.8515625 - 18.6875 * e), vec3(1.0 / 0.1593017578125));
  }
  vec3 rgb = nits / referenceNits;
  if (convertPrimaries) rgb = primaries * rgb;
  rgb = max(rgb, 0.0);
  float m = max(max(rgb.r, rgb.g), rgb.b);
  float gain = m > 0.0 ? min(1.0, hableCurve(m) / hableCurve(peakNits / referenceNits)) / m : 0.0;
  color = vec4(rgb * gain, 1.0);
}`;

// Passes 2-4: sample the linear image at the output size (trilinear, so a
// downscale averages in linear light), apply the BT.709 OETF and write one
// limited-range 8-bit plane, four samples per RGBA texel.
const ENCODE = `#version 300 es
precision highp float;
uniform sampler2D image;
uniform vec2 planeSize, outputSize;
uniform float lod;
uniform int plane; // 0 Y, 1 Cb, 2 Cr
out vec4 color;
vec3 oetf(vec3 l) {
  l = clamp(l, 0.0, 1.0);
  return mix(4.5 * l, 1.099 * pow(l, vec3(0.45)) - 0.099, step(0.018, l));
}
float sampleAt(float x, float y) {
  vec2 point = plane == 0 ? vec2(x + 0.5, y + 0.5) : vec2(2.0 * x + 0.5, 2.0 * y + 1.0);
  vec3 rgb = oetf(textureLod(image, point / outputSize, lod).rgb);
  float luma = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
  float value = plane == 0 ? 16.0 + 219.0 * luma : plane == 1 ? 128.0 + 224.0 * (rgb.b - luma) / 1.8556 : 128.0 + 224.0 * (rgb.r - luma) / 1.5748;
  return clamp(floor(value + 0.5), 0.0, 255.0) / 255.0;
}
void main() {
  float x = floor(gl_FragCoord.x) * 4.0, y = floor(gl_FragCoord.y);
  color = vec4(sampleAt(x, y), sampleAt(min(x + 1.0, planeSize.x - 1.0), y), sampleAt(min(x + 2.0, planeSize.x - 1.0), y), sampleAt(min(x + 3.0, planeSize.x - 1.0), y));
}`;

export interface HdrFrameSource {
  format: string | null;
  codedWidth: number;
  codedHeight: number;
  visibleRect: { left: number; top: number; width: number; height: number } | null;
  colorSpace: { fullRange?: boolean | null; matrix?: string | null; primaries?: string | null; transfer?: string | null };
  allocationSize(): number;
  copyTo(destination: Uint8Array): Promise<Array<{ offset: number; stride: number }>>;
}

export interface I420Frame {
  data: Uint8Array;
  layout: Array<{ offset: number; stride: number }>;
  width: number;
  height: number;
}

function compile(gl: WebGL2RenderingContext, vertex: string, fragment: string) {
  const program = gl.createProgram()!;
  for (const [type, source] of [[gl.VERTEX_SHADER, vertex], [gl.FRAGMENT_SHADER, fragment]] as const) {
    const shader = gl.createShader(type)!;
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) ?? "shader");
    gl.attachShader(program, shader);
  }
  gl.bindAttribLocation(program, 0, "position");
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? "link");
  return program;
}

/** Column-major mat3 from a row-major 3×3 list. */
const columnMajor = (m: readonly number[]) => [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];

function yuvMatrix(matrix: string | null | undefined) {
  const [kr, kb] = matrix === "bt709" ? [0.2126, 0.0722] : [0.2627, 0.0593];
  const kg = 1 - kr - kb;
  return { kr, kg, kb, rows: [1, 0, 2 * (1 - kr), 1, -2 * kb * (1 - kb) / kg, -2 * kr * (1 - kr) / kg, 1, 2 * (1 - kb), 0] };
}

/** Converts decoded HDR frames to SDR BT.709 I420 at `width`×`height`. Throws
 * when WebGL2, float render targets or the frame format are unavailable;
 * callers then keep the original file. */
export class HdrToneMapper {
  private readonly gl: WebGL2RenderingContext;
  private readonly decode: WebGLProgram;
  private readonly encode: WebGLProgram;
  private readonly planes: WebGLTexture[];
  private readonly linear: WebGLTexture;
  private readonly linearFramebuffer: WebGLFramebuffer;
  private readonly outputs: Array<{ texture: WebGLTexture; framebuffer: WebGLFramebuffer; texels: number; rows: number; size: [number, number] }>;
  private linearSize: [number, number] = [0, 0];
  private staging = new Uint8Array(0);
  readonly layout: Array<{ offset: number; stride: number }>;
  private readonly byteLength: number;

  readonly width: number;
  readonly height: number;
  private readonly transfer: HdrTransfer;

  constructor(width: number, height: number, transfer: HdrTransfer) {
    this.width = width;
    this.height = height;
    this.transfer = transfer;
    if (width % 2 || height % 2) throw new Error("I420 output needs even dimensions");
    const canvas = new OffscreenCanvas(1, 1);
    const gl = canvas.getContext("webgl2", { antialias: false, depth: false, stencil: false, premultipliedAlpha: false });
    if (!gl || !gl.getExtension("EXT_color_buffer_float")) throw new Error("WebGL2 with float render targets is unavailable");
    this.gl = gl;
    this.decode = compile(gl, VERTEX, DECODE);
    this.encode = compile(gl, VERTEX, ENCODE);
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    this.planes = [0, 1, 2].map(() => this.texture(gl.NEAREST, gl.NEAREST));
    this.linear = this.texture(gl.LINEAR_MIPMAP_LINEAR, gl.LINEAR);
    this.linearFramebuffer = gl.createFramebuffer()!;
    const chroma: [number, number] = [width / 2, height / 2];
    let offset = 0;
    this.layout = [];
    this.outputs = [[width, height], chroma, chroma].map(([w, h]) => {
      const texels = Math.ceil(w / 4);
      const texture = this.texture(gl.NEAREST, gl.NEAREST);
      gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, texels, h);
      const framebuffer = gl.createFramebuffer()!;
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      this.layout.push({ offset, stride: texels * 4 });
      offset += texels * 4 * h;
      return { texture, framebuffer, texels, rows: h, size: [w, h] as [number, number] };
    });
    this.byteLength = offset;
  }

  private texture(min: number, mag: number) {
    const gl = this.gl;
    const texture = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, min);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, mag);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return texture;
  }

  private ensureLinear(width: number, height: number) {
    if (this.linearSize[0] === width && this.linearSize[1] === height) return;
    const gl = this.gl;
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.linear);
    const levels = Math.floor(Math.log2(Math.max(width, height))) + 1;
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, width, height, 0, gl.RGBA, gl.HALF_FLOAT, null);
    for (let level = 1, w = width, h = height; level < levels; level++) {
      w = Math.max(1, w >> 1); h = Math.max(1, h >> 1);
      gl.texImage2D(gl.TEXTURE_2D, level, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.linearFramebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.linear, 0);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error("RGBA16F is not renderable");
    this.linearSize = [width, height];
  }

  private upload(index: number, bytes: Uint8Array, offset: number, stride: number, width: number, height: number, depth: number, channels: 1 | 2) {
    const gl = this.gl;
    gl.activeTexture(gl.TEXTURE0 + index);
    gl.bindTexture(gl.TEXTURE_2D, this.planes[index]);
    const wide = depth > 8;
    const data = wide ? new Uint16Array(bytes.buffer, bytes.byteOffset + offset, (stride * (height - 1)) / 2 + width * channels) : bytes.subarray(offset, offset + stride * (height - 1) + width * channels);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.pixelStorei(gl.UNPACK_ROW_LENGTH, wide ? stride / 2 / channels : stride / channels);
    const internal = channels === 2 ? (wide ? gl.RG16UI : gl.RG8UI) : (wide ? gl.R16UI : gl.R8UI);
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, width, height, 0, channels === 2 ? gl.RG_INTEGER : gl.RED_INTEGER, wide ? gl.UNSIGNED_SHORT : gl.UNSIGNED_BYTE, data);
    gl.pixelStorei(gl.UNPACK_ROW_LENGTH, 0);
  }

  /** Tone maps one decoded frame into a reused I420 buffer. */
  async map(frame: HdrFrameSource): Promise<I420Frame> {
    const info = frame.format ? FORMATS[frame.format] : undefined;
    if (!info) throw new Error(`Unsupported frame format ${frame.format}`);
    const size = frame.allocationSize();
    if (this.staging.length < size) this.staging = new Uint8Array(size);
    const layout = await frame.copyTo(this.staging);
    const gl = this.gl;
    const { codedWidth: w, codedHeight: h } = frame;
    const rect = frame.visibleRect ?? { left: 0, top: 0, width: w, height: h };
    this.ensureLinear(rect.width, rect.height);
    const cw = info.chroma[0] ? Math.ceil(w / 2) : w, ch = info.chroma[1] ? Math.ceil(h / 2) : h;
    this.upload(0, this.staging, layout[0].offset, layout[0].stride, w, h, info.depth, 1);
    if (info.nv12) this.upload(1, this.staging, layout[1].offset, layout[1].stride, cw, ch, 8, 2);
    else {
      this.upload(1, this.staging, layout[1].offset, layout[1].stride, cw, ch, info.depth, 1);
      this.upload(2, this.staging, layout[2].offset, layout[2].stride, cw, ch, info.depth, 1);
    }

    gl.useProgram(this.decode);
    const u = (name: string) => gl.getUniformLocation(this.decode, name);
    gl.uniform1i(u("planeY"), 0); gl.uniform1i(u("planeU"), 1); gl.uniform1i(u("planeV"), 2);
    gl.uniform1i(u("nv12"), info.nv12 ? 1 : 0);
    gl.uniform2i(u("chromaShift"), info.chroma[0], info.chroma[1]);
    gl.uniform2i(u("origin"), rect.left, rect.top);
    const scale = 2 ** (info.depth - 8), max = 2 ** info.depth - 1;
    gl.uniform4fv(u("range"), frame.colorSpace.fullRange ? [0, max, 2 ** (info.depth - 1), max] : [16 * scale, 219 * scale, 128 * scale, 224 * scale]);
    const yuv = yuvMatrix(frame.colorSpace.matrix);
    gl.uniformMatrix3fv(u("yuvToRgb"), false, columnMajor(yuv.rows));
    gl.uniformMatrix3fv(u("primaries"), false, columnMajor(BT2020_TO_BT709));
    gl.uniform1i(u("convertPrimaries"), frame.colorSpace.primaries === "bt709" ? 0 : 1);
    gl.uniform1i(u("hlg"), this.transfer === "hlg" ? 1 : 0);
    gl.uniform1f(u("lumaR"), yuv.kr); gl.uniform1f(u("lumaG"), yuv.kg); gl.uniform1f(u("lumaB"), yuv.kb);
    gl.uniform1f(u("referenceNits"), SDR_REFERENCE_NITS);
    gl.uniform1f(u("peakNits"), HDR_PEAK_NITS);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.linearFramebuffer);
    gl.viewport(0, 0, rect.width, rect.height);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.linear);
    gl.generateMipmap(gl.TEXTURE_2D);

    gl.useProgram(this.encode);
    const e = (name: string) => gl.getUniformLocation(this.encode, name);
    gl.uniform1i(e("image"), 0);
    gl.uniform2f(e("outputSize"), this.width, this.height);
    const downscale = Math.max(rect.width / this.width, rect.height / this.height);
    const data = new Uint8Array(this.byteLength);
    this.outputs.forEach((output, plane) => {
      gl.uniform1i(e("plane"), plane);
      gl.uniform2f(e("planeSize"), output.size[0], output.size[1]);
      // Luma samples one output pixel; chroma averages a 2×2 block.
      gl.uniform1f(e("lod"), Math.max(0, Math.log2(downscale) + (plane ? 1 : 0)));
      gl.bindFramebuffer(gl.FRAMEBUFFER, output.framebuffer);
      gl.viewport(0, 0, output.texels, output.rows);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.readPixels(0, 0, output.texels, output.rows, gl.RGBA, gl.UNSIGNED_BYTE, data, this.layout[plane].offset);
    });
    return { data, layout: this.layout, width: this.width, height: this.height };
  }

  dispose() {
    this.gl.getExtension("WEBGL_lose_context")?.loseContext();
  }
}
