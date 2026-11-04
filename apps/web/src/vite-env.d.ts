declare const __LATEST_CHANGES__: import("./release-highlights").LatestChange[];
declare const __SOURCE_REVISION__: string;

declare module "libheif-js/libheif-wasm/libheif-bundle.mjs" {
  /** libheif's Emscripten module with its WASM inlined (see chat/heic-worker.ts). */
  const createLibheif: () => Promise<{ HeifDecoder: new () => { decode(bytes: Uint8Array): unknown[] } }>;
  export default createLibheif;
}
