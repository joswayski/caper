class CaperRecorder extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.maximumFrames = Math.max(0, Math.floor(options.processorOptions?.maximumFrames ?? sampleRate * 5));
    this.frames = 0;
    this.stopped = false;
    this.port.onmessage = ({ data }) => { if (data === "stop") this.stopped = true; };
  }

  process(inputs, outputs) {
    const output = outputs[0]?.[0];
    if (output) output.fill(0);
    if (this.stopped) return false;
    const input = inputs[0]?.[0];
    if (input && this.frames < this.maximumFrames) {
      const length = Math.min(input.length, this.maximumFrames - this.frames);
      const samples = input.slice(0, length);
      this.frames += length;
      this.port.postMessage({ type: "samples", samples }, [samples.buffer]);
    }
    if (this.frames >= this.maximumFrames) {
      this.port.postMessage({ type: "done" });
      this.stopped = true;
      return false;
    }
    return true;
  }
}

registerProcessor("caper-recorder", CaperRecorder);
