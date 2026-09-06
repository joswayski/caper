const HOP = 480;
const STARTUP_HOPS = 3;
const MAX_BACKLOG_HOPS = 8;

class DpdfnetProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.input = new Float32Array(HOP);
    this.inputOffset = 0;
    this.output = [];
    this.outputOffset = 0;
    this.inFlight = 0;
    this.started = false;
    this.stopped = false;
    this.port.onmessage = ({ data }) => {
      if (data === 'stop') { this.stopped = true; this.port.close(); return; }
      if (data?.type === 'failed') { this.fail(); return; }
      if (data?.type === 'output') {
        this.inFlight--;
        if (this.output.length >= MAX_BACKLOG_HOPS) { this.fail(); return; }
        this.output.push(new Float32Array(data.samples));
      }
    };
  }
  fail() {
    if (!this.stopped) this.port.postMessage('failed');
    this.stopped = true;
  }
  process(inputs, outputs) {
    if (this.stopped) return false;
    const source = inputs[0]?.[0];
    const target = outputs[0]?.[0];
    if (!target) return true;
    if (!source) { target.fill(0); return true; }
    for (let i = 0; i < source.length; i++) {
      this.input[this.inputOffset++] = source[i];
      if (this.inputOffset === HOP) {
        if (this.inFlight >= MAX_BACKLOG_HOPS) {
          this.fail();
          return false;
        }
        const samples = this.input;
        this.input = new Float32Array(HOP);
        this.inputOffset = 0;
        this.inFlight++;
        this.port.postMessage({ type: 'process', samples: samples.buffer }, [samples.buffer]);
      }
      if (!this.started && this.output.length >= STARTUP_HOPS) this.started = true;
      const block = this.started ? this.output[0] : undefined;
      if (this.started && !block) {
        this.fail();
        return false;
      }
      target[i] = block ? block[this.outputOffset++] : 0;
      if (block && this.outputOffset === HOP) { this.output.shift(); this.outputOffset = 0; }
    }
    return true;
  }
}
registerProcessor('caper-dpdfnet2', DpdfnetProcessor);
