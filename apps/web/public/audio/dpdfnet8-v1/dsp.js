// DPDFNet's streaming STFT/ISTFT. Inference stays in the Worker (never the audio callback).
const N = 960;
const HOP = 480;
const BINS = 481;
const window = Float32Array.from({ length: N }, (_, i) =>
  Math.sin(Math.PI / 2 * Math.sin(Math.PI * (i + 0.5) / N) ** 2));

function makePlan(n, inverse) {
  if (n === 1) return { n, output: new Float64Array(2), children: [] };
  const radix = n % 2 === 0 ? 2 : n % 3 === 0 ? 3 : n % 5 === 0 ? 5 : n;
  const size = n / radix;
  const roots = new Float64Array(n * radix * 2);
  for (let index = 0; index < n; index++) for (let r = 0; r < radix; r++) {
    const angle = (inverse ? 1 : -1) * 2 * Math.PI * r * index / n;
    const at = (index * radix + r) * 2;
    roots[at] = Math.cos(angle); roots[at + 1] = Math.sin(angle);
  }
  return { n, radix, size, roots, output: new Float64Array(n * 2),
    children: Array.from({ length: radix }, () => makePlan(size, inverse)) };
}

function execute(plan, input, offset = 0, stride = 1) {
  if (plan.n === 1) {
    plan.output[0] = input[offset * 2]; plan.output[1] = input[offset * 2 + 1];
    return plan.output;
  }
  for (let r = 0; r < plan.radix; r++) execute(plan.children[r], input, offset + r * stride, stride * plan.radix);
  for (let q = 0; q < plan.size; q++) for (let s = 0; s < plan.radix; s++) {
    const index = q + plan.size * s;
    let re = 0, im = 0;
    for (let r = 0; r < plan.radix; r++) {
      const part = plan.children[r].output;
      const pr = part[q * 2], pi = part[q * 2 + 1];
      const at = (index * plan.radix + r) * 2;
      const cr = plan.roots[at], ci = plan.roots[at + 1];
      re += pr * cr - pi * ci; im += pr * ci + pi * cr;
    }
    plan.output[index * 2] = re; plan.output[index * 2 + 1] = im;
  }
  return plan.output;
}

const forwardPlan = makePlan(N, false);
const inversePlan = makePlan(N, true);
function fft(input, inverse = false) {
  const output = execute(inverse ? inversePlan : forwardPlan, input);
  if (inverse) for (let i = 0; i < output.length; i++) output[i] /= N;
  return output;
}

export class DpdfnetStream {
  constructor(run, initialState) {
    this.run = run;
    this.state = initialState;
    this.analysis = new Float32Array(N);
    this.ola = new Float32Array(N);
    this.time = new Float64Array(N * 2);
    this.spectrum = new Float64Array(N * 2);
    this.primed = false;
  }
  async process(hop) {
    this.analysis.copyWithin(0, HOP);
    this.analysis.set(hop, HOP);
    for (let i = 0; i < N; i++) this.time[i * 2] = this.analysis[i] * window[i];
    const transformed = fft(this.time);
    const spec = new Float32Array(BINS * 2);
    for (let i = 0; i < spec.length; i++) spec[i] = transformed[i];
    const result = await this.run(spec, this.state);
    this.state = result.state;
    this.spectrum.fill(0);
    this.spectrum.set(result.spec);
    for (let i = 1; i < BINS - 1; i++) {
      this.spectrum[(N - i) * 2] = result.spec[i * 2];
      this.spectrum[(N - i) * 2 + 1] = -result.spec[i * 2 + 1];
    }
    const frame = fft(this.spectrum, true);
    const output = new Float32Array(HOP);
    for (let i = 0; i < HOP; i++) output[i] = this.ola[i] + frame[i * 2] * window[i];
    this.ola.copyWithin(0, HOP);
    this.ola.fill(0, HOP);
    for (let i = 0; i < HOP; i++) this.ola[i] += frame[(i + HOP) * 2] * window[i + HOP];
    if (!this.primed) { this.primed = true; output.fill(0); }
    return output;
  }
}
