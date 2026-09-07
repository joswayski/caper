// Run against Vite: node scripts/test-mic-playback.mjs http://localhost:<port>
// Requires installed agent-browser. Real Chromium/MediaRecorder, synthetic input;
// this component regression test does NOT claim Cloudflare or hardware validation.
import { execFileSync } from "node:child_process";

const origin = new URL(process.argv[2] ?? "http://localhost:5174");
if (!["localhost", "127.0.0.1"].includes(origin.hostname)) throw new Error("Use a local Vite server, not production.");
const session = `mic-playback-${process.pid}`;
function browser(...args) {
  return execFileSync("agent-browser", ["--session", session, ...args], { encoding: "utf8", timeout: 35_000 });
}
function evaluate(code) { return browser("eval", `(async()=>{${code}})()`); }

try {
  browser("open", new URL("/live", origin).href);
  browser("wait", "--fn", "!!document.querySelector('.join-card button:not(:disabled)') || document.body.textContent.includes('Voice is currently unavailable')");
  evaluate(`
    const {default: React} = await import('/node_modules/.vite/deps/react.js');
    const {default: {createRoot}} = await import('/node_modules/.vite/deps/react-dom_client.js');
    const {default: MicPlayback} = await import('/src/pages/MicPlayback.tsx');
    window.assert = (condition, message) => { if (!condition) throw new Error(message); };
    window.wait = ms => new Promise(resolve => setTimeout(resolve, ms));
    window.until = async predicate => {
      for (let i = 0; i < 100; i++) { if (predicate()) return; await wait(50); }
      throw new Error('Timed out waiting for component state');
    };
    window.button = name => [...document.querySelectorAll('button')].find(b => b.textContent === name);
    window.audio = () => document.querySelector('audio[controls]');
    window.recorders = [];
    const Recorder = MediaRecorder;
    window.MediaRecorder = class extends Recorder {
      constructor(...args) { super(...args); recorders.push(this); }
    };
    document.querySelector('main').hidden = true;
    const label = document.createElement('p');
    label.textContent = 'Synthetic component regression test — not Cloudflare';
    document.body.append(label);
    const mount = document.createElement('div'); document.body.append(mount);
    const root = createRoot(mount);
    const context = new AudioContext();
    const oscillator = context.createOscillator();
    window.gain = context.createGain(); gain.gain.value = 0.2;
    const destination = context.createMediaStreamDestination();
    oscillator.connect(gain).connect(destination); oscillator.start();
    window.stream = destination.stream;
    window.render = (props = {}) => root.render(React.createElement(MicPlayback, {stream, output: '', ...props}));
    window.cleanup = () => { root.unmount(); stream.getTracks().forEach(t=>t.stop()); return context.close(); };
    const start = document.createElement('button'); start.textContent = 'Enable synthetic audio';
    start.onclick = async () => { await context.resume(); start.remove(); render(); };
    document.body.append(start);
    return 'ready';
  `);
  browser("find", "role", "button", "click", "--name", "Enable synthetic audio");
  console.log(evaluate(`
    await until(() => button('Record microphone'));
    assert(recorders.length === 0, 'must not record on mount');
    button('Record microphone').click();
    await wait(1200);
    button('Stop & play back').click();
    await until(() => audio()?.readyState >= 2);
    const url = audio().src;
    const context = new AudioContext();
    const decoded = await context.decodeAudioData(await (await fetch(url)).arrayBuffer());
    const peak = decoded.getChannelData(0).reduce((m,v)=>Math.max(m, Math.abs(v)), 0);
    assert(peak > 0.1 && decoded.duration > 0.8, 'recording must decode to the source signal');
    await context.close();
    render({status: 'Updated connection diagnostics'});
    await wait(1500);
    assert(audio().src === url && recorders.length === 1, 'stop or parent updates restarted recording');
    audio().currentTime = 0; await audio().play(); await wait(200);
    assert(audio().currentTime > 0 && recorders.length === 1, 'replay must advance without recording');
    audio().pause(); await wait(50);
    assert(document.body.textContent.includes('Recording ready.'), 'pause status must not remain playing');
    return 'PASS explicit recording, decoded signal, stop, rerender, replay, pause';
  `));
  console.log(evaluate(`
    const old = audio().src;
    button('Test again').click(); await wait(300);
    assert(recorders.length === 2, 'Test again must explicitly start another recording');
    window.stream = new MediaStream(stream.getTracks()); render();
    await until(() => button('Record microphone'));
    assert(recorders[1].state === 'inactive' && !audio(), 'stream change must cancel capture and clear playback');
    assert(recorders.length === 2 && stream.getAudioTracks()[0].readyState === 'live', 'must not restart or stop borrowed track');
    let revoked = false; try { await fetch(old); } catch { revoked = true; }
    assert(revoked, 'old recording URL must be revoked');
    gain.gain.value = 0;
    await wait(300); // Drain the source's already-rendered audio before testing silence.
    button('Record microphone').click(); await wait(1000); button('Stop & play back').click();
    await until(() => audio() && document.body.textContent.includes('No audible signal'));
    assert(recorders.length === 3, 'silent recording must not restart');
    return 'PASS test again, stream replacement, cancellation, URL cleanup, silent recording warning';
  `));
  console.log(evaluate(`
    gain.gain.value = 0.2;
    button('Test again').click();
    await wait(11000);
    await until(() => audio());
    assert(recorders.length === 4 && recorders[3].state === 'inactive', 'ten-second recording limit');
    const url = audio().src;
    const originalSink = HTMLMediaElement.prototype.setSinkId;
    HTMLMediaElement.prototype.setSinkId = async function() { throw new DOMException('Denied', 'NotAllowedError'); };
    render({output:'unavailable'});
    await until(() => document.body.textContent.includes('Audio output unavailable'));
    assert(audio().src === url && recorders.length === 4, 'output failure must preserve recording');
    HTMLMediaElement.prototype.setSinkId = originalSink;
    render(); await until(() => !document.body.textContent.includes('Audio output unavailable'));
    const originalPlay = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function() {
      return this.hasAttribute('controls') ? Promise.reject(new DOMException('Blocked', 'NotAllowedError')) : originalPlay.call(this);
    };
    button('Test again').click(); await wait(1000); button('Stop & play back').click();
    await until(() => audio() && document.body.textContent.includes('Recording ready.'));
    assert(!document.body.textContent.includes('Audio output unavailable'), 'autoplay blocking is not an output-device failure');
    HTMLMediaElement.prototype.play = originalPlay;
    button('Test again').click(); await wait(100); await cleanup();
    assert(recorders.every(r => r.state === 'inactive'), 'unmount must stop every recorder');
    return 'PASS automatic limit, output denial/recovery, autoplay blocking, unmount cleanup';
  `));
} catch (error) {
  console.error(browser("get", "text", "body"));
  throw error;
} finally {
  browser("close");
}
