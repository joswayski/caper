// Real browser audio/ONNX/RNNoise with synthetic noise and an intentionally stalled
// DPDFNet worker. No hardware microphone, production API, or SFU is used.
// node scripts/test-noise-fallback.mjs http://localhost:5174
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';

const origin = new URL(process.argv[2] ?? 'http://localhost:5174');
assert.ok(['localhost', '127.0.0.1'].includes(origin.hostname));
const args = ['--session', 'noise-fallback', '--args', '--autoplay-policy=no-user-gesture-required'];
const browser = (...command) => JSON.parse(execFileSync('agent-browser', [...args, ...command, '--json'], { encoding: 'utf8', timeout: 90000 }));
function evaluate(code) {
  const result = browser('eval', `(async()=>{${code}})()`);
  assert.ok(result.success, JSON.stringify(result));
  return result.data.result;
}
try {
  browser('open', new URL('/spaces', origin).href);
  browser('wait', '.spaces-state, .chat-panel, .spaces-empty');
  browser('set', 'viewport', '1280', '900', '2');
  browser('click', 'body');
  evaluate(`
    window.wait = ms => new Promise(r => setTimeout(r, ms));
    window.assert = (value, message) => { if (!value) throw new Error(message); };
    const { captureMicrophone } = await import('/src/media/microphone.ts');
    const WorkerClass = Worker;
    window.Worker = class extends WorkerClass {
      postMessage(message, ...rest) {
        // Let the real model initialize, then withhold live hops to force overload.
        if (message?.type !== 'process') super.postMessage(message, ...rest);
      }
    };
    window.context = new AudioContext({sampleRate: 48000});
    await context.resume();
    const buffer = context.createBuffer(1, 48000, 48000);
    let seed = 42;
    buffer.getChannelData(0).set(Float32Array.from({length: 48000}, () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return (seed / 4294967296 - .5) * .2;
    }));
    const source = context.createBufferSource(); source.buffer = buffer; source.loop = true;
    window.raw = context.createMediaStreamDestination(); source.connect(raw); source.start();
    navigator.mediaDevices.getUserMedia = async () => raw.stream;
    window.capturePromise = captureMicrophone(undefined, 'dpdfnet8', new AbortController().signal, () => {}, 'headphones')
      .then(value => {window.capture = value;}).catch(error => {window.captureError = String(error);});
  `);
  browser('wait', '--fn', '!!window.capture || !!window.captureError', '--timeout', '70000');
  console.log(evaluate(`
    assert(window.capture, window.captureError);
    const track = capture.track, natural = capture.naturalTrack;
    for (let i = 0; i < 200 && !capture.status.includes('RNNoise active'); i++) await wait(50);
    assert(capture.status.includes('RNNoise active'), capture.status);
    assert(capture.track === track && capture.naturalTrack === natural, 'fallback changed track identity');
    const {recordReceivedAudio} = await import('/src/media/recording.ts');
    const recordings = [recordReceivedAudio(raw.stream), recordReceivedAudio(new MediaStream([natural]))];
    await wait(2000); recordings.forEach(r => r.finish());
    const blobs = await Promise.all(recordings.map(r => r.result));
    const buffers = await Promise.all(blobs.map(async b => context.decodeAudioData(await b.arrayBuffer())));
    const rms = buffers.map(b => {
      const samples = b.getChannelData(0).slice(4800);
      return Math.sqrt(samples.reduce((sum, v) => sum + v*v, 0) / samples.length);
    });
    assert(rms[0] > .01 && rms[1] > 0, 'must test real nonzero audio, not silence');
    assert(rms[1] < rms[0] * .5, 'fallback must attenuate noise by at least 6 dB');
    window.filteredStream = new MediaStream([natural]);
    return {status: capture.status, reductionDb: 20 * Math.log10(rms[0] / rms[1])};
  `));
  evaluate(`
    const {default: React} = await import('/node_modules/.vite/deps/react.js');
    const {default: {createRoot}} = await import('/node_modules/.vite/deps/react-dom_client.js');
    const {default: MicPlayback} = await import('/src/pages/MicPlayback.tsx');
    document.querySelectorAll('main').forEach(e => e.style.display = 'none');
    const mount = document.createElement('div'); document.body.append(mount);
    createRoot(mount).render(React.createElement('dialog', {className: 'audio-dialog', ref: e => {if(e && !e.open) e.showModal();}},
      React.createElement('p', null, 'Synthetic microphone fixture — real RNNoise fallback'),
      React.createElement(MicPlayback, {stream: filteredStream, output: '', processingStrength: 50, noiseStatus: capture.status, onProcessingStrengthChange: () => {}})));
  `);
  browser('wait', '.mic-test-card');
  for (const width of [1280, 390]) {
    browser('set', 'viewport', String(width), '900', '2');
    browser('click', '[aria-label="About On-device noise cancellation"]');
    evaluate(`
      const tip = document.querySelector('#noise-cancellation-detail');
      assert(tip.matches(':popover-open'), 'tooltip must open on click');
      const box = tip.getBoundingClientRect();
      assert(box.left >= 0 && box.right <= innerWidth && box.top >= 0 && box.bottom <= innerHeight, 'tooltip clipped by viewport');
      assert(document.elementFromPoint(box.left + 10, box.top + 10) === tip, 'tooltip must be above the modal');
    `);
    browser('press', 'Escape');
    evaluate(`assert(!document.querySelector(':popover-open') && document.querySelector('dialog[open]'), 'Escape should dismiss only the tooltip');`);
  }
  console.log('PASS: real RNNoise fallback attenuates synthetic noise; stable tracks; desktop/narrow top-layer tooltip and Escape. Not hardware/SFU validation.');
} catch (error) {
  console.error(evaluate(`return {context: window.context?.state, capture: window.capture?.status};`));
  throw error;
} finally {
  browser('close');
}
