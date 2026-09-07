export class DpdfnetStream {
  constructor(
    run: (spec: Float32Array, state: Float32Array) => Promise<{ spec: Float32Array; state: Float32Array }>,
    initialState: Float32Array,
  );
  process(hop: Float32Array): Promise<Float32Array>;
}
