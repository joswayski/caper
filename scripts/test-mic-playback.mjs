// Run against Vite: node scripts/test-mic-playback.mjs http://localhost:<port>
// Requires installed agent-browser. Real Chromium/MediaRecorder, synthetic input;
// this component regression test does NOT claim Cloudflare or hardware validation.
import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const origin = new URL(process.argv[2] ?? "http://localhost:5174");
if (!["localhost", "127.0.0.1"].includes(origin.hostname)) throw new Error("Use a local Vite server, not production.");
const session = `mic-playback-${process.pid}`;
function browser(...args) {
  return execFileSync("agent-browser", ["--session", session, ...args], { encoding: "utf8", timeout: 35_000 });
}
function evaluate(code) {
  return browser("eval", `(async()=>{${code}})()`);
}

try {
  // Any page works: the fixture hides the app shell and mounts the real component.
  browser("open", new URL("/privacy", origin).href);
  browser("wait", "main");
  evaluate(`
    const {default: React} = await import('/node_modules/.vite/deps/react.js');
    const {default: {createRoot}} = await import('/node_modules/.vite/deps/react-dom_client.js');
    const {default: MicPlayback} = await import('/src/pages/MicPlayback.tsx');
    const {MAX_RECORDING_SECONDS} = await import('/src/media/recording.ts');
    window.maxRecordingSeconds = MAX_RECORDING_SECONDS;
    window.assert = (condition, message) => { if (!condition) throw new Error(message); };
    window.wait = ms => new Promise(resolve => setTimeout(resolve, ms));
    window.until = async (predicate, message = 'Timed out waiting for component state') => {
      for (let i = 0; i < 200; i++) { if (predicate()) return; await wait(50); }
      throw new Error(message);
    };
    window.button = name => [...document.querySelectorAll('button')].find(b => b.textContent === name);
    // "Test microphone" is disabled while the enhanced comparison is prepared.
    window.idle = () => button('Test microphone') && !button('Test microphone').disabled;
    window.audio = (label = 'Natural') => document.querySelector('audio[aria-label="' + label + ' audio sample"]');
    window.recorders = [];
    const Recorder = MediaRecorder;
    window.MediaRecorder = class extends Recorder {
      constructor(...args) { super(...args); recorders.push(this); }
    };
    document.querySelector('main').hidden = true;
    const label = document.createElement('p');
    label.textContent = 'Synthetic component regression test - not Cloudflare';
    document.body.append(label);
    const mount = document.createElement('div'); document.body.append(mount);
    const root = createRoot(mount);
    const context = new AudioContext();
    const oscillator = context.createOscillator();
    window.gain = context.createGain(); gain.gain.value = 0.2;
    const destination = context.createMediaStreamDestination();
    oscillator.connect(gain).connect(destination); oscillator.start();
    window.stream = destination.stream;
    window.strength = 0;
    window.render = (props = {}) => root.render(React.createElement(MicPlayback, {
      stream, output: '', processingStrength: strength,
      onProcessingStrengthChange: value => { strength = value; render(props); },
      ...props,
    }));
    window.cleanup = () => { root.unmount(); stream.getTracks().forEach(t=>t.stop()); return context.close(); };
    window.decode = async url => {
      const decoder = new AudioContext();
      try { return await decoder.decodeAudioData(await (await fetch(url)).arrayBuffer()); }
      finally { await decoder.close(); }
    };
    window.peak = buffer => buffer.getChannelData(0).reduce((m,v)=>Math.max(m, Math.abs(v)), 0);
    const start = document.createElement('button'); start.textContent = 'Enable synthetic audio';
    start.onclick = async () => { await context.resume(); start.remove(); render(); };
    document.body.append(start);
    return 'ready';
  `);
  browser("find", "role", "button", "click", "--name", "Enable synthetic audio");
  console.log(
    evaluate(`
    await until(() => button('Test microphone'));
    assert(recorders.length === 0, 'must not record on mount');
    button('Test microphone').click();
    await until(() => button('Stop recording'));
    await wait(1200);
    button('Stop recording').click();
    await until(() => audio()?.readyState >= 2);
    const url = audio().src;
    const natural = await decode(url);
    assert(peak(natural) > 0.1 && natural.duration > 0.8, 'recording must decode to the source signal');
    await until(() => audio('Enhanced')?.readyState >= 2 && idle(), 'enhanced comparison was not prepared');
    const enhanced = await decode(audio('Enhanced').src);
    assert(peak(enhanced) > 0.05 && Math.abs(enhanced.duration - natural.duration) < 0.1, 'enhanced comparison must keep the signal');
    render();
    await wait(1500);
    assert(audio().src === url && recorders.length === 1, 'stop or parent updates restarted recording');
    // The natural sample autoplays, then the enhanced one; replay after both finish.
    await until(() => audio().ended && audio('Enhanced').ended, 'natural then enhanced samples must autoplay');
    audio().currentTime = 0; await audio().play(); await wait(200);
    assert(audio().currentTime > 0 && recorders.length === 1, 'replay must advance without recording');
    audio().pause(); await wait(50);
    assert(idle() && !button('Stop recording') && recorders.length === 1, 'pause must leave the test idle, not recording');
    return 'PASS explicit recording, decoded natural/enhanced signal, stop, rerender, replay, pause';
  `),
  );
  console.log(
    evaluate(`
    const old = audio().src;
    button('Test microphone').click(); await wait(300);
    assert(recorders.length === 2 && button('Stop recording'), 'Test microphone must explicitly start another recording');
    window.stream = new MediaStream(stream.getTracks()); render();
    await until(() => button('Test microphone'));
    assert(recorders[1].state === 'inactive' && !audio(), 'stream change must cancel capture and clear playback');
    assert(recorders.length === 2 && stream.getAudioTracks()[0].readyState === 'live', 'must not restart or stop borrowed track');
    let revoked = false; try { await fetch(old); } catch { revoked = true; }
    assert(revoked, 'old recording URL must be revoked');
    gain.gain.value = 0;
    await wait(300); // Drain the source's already-rendered audio before testing silence.
    button('Test microphone').click(); await wait(1000); button('Stop recording').click();
    await until(() => audio() && document.body.textContent.includes('No audible signal'));
    assert(recorders.length === 3, 'silent recording must not restart');
    await until(idle);
    return 'PASS test again, stream replacement, cancellation, URL cleanup, silent recording warning';
  `),
  );
  evaluate(`
    gain.gain.value = 0.2;
    button('Test microphone').click();
    await until(() => button('Stop recording'));
    return 'recording';
  `);
  const limit = Number(evaluate("return maxRecordingSeconds"));
  await sleep((limit + 1) * 1000);
  console.log(
    evaluate(`
    await until(() => audio() && idle(), 'recording did not stop at the limit');
    assert(recorders.length === 4 && recorders[3].state === 'inactive', 'recording length limit');
    const url = audio().src;
    const sink = [HTMLMediaElement.prototype.setSinkId, AudioContext.prototype.setSinkId];
    const deny = async function() { throw new DOMException('Denied', 'NotAllowedError'); };
    HTMLMediaElement.prototype.setSinkId = deny; AudioContext.prototype.setSinkId = deny;
    render({output:'unavailable'});
    await until(() => document.body.textContent.includes('Audio output unavailable'));
    assert(audio().src === url && recorders.length === 4, 'output failure must preserve recording');
    [HTMLMediaElement.prototype.setSinkId, AudioContext.prototype.setSinkId] = sink;
    render(); await until(() => !document.body.textContent.includes('Audio output unavailable'), 'choosing a working output must clear the output error');
    assert(audio().src === url && recorders.length === 4, 'output recovery must preserve recording');
    const originalPlay = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function() {
      return this.hasAttribute('controls') ? Promise.reject(new DOMException('Blocked', 'NotAllowedError')) : originalPlay.call(this);
    };
    button('Test microphone').click(); await wait(1000); button('Stop recording').click();
    await until(() => audio() && idle());
    assert(!document.body.textContent.includes('Audio output unavailable'), 'autoplay blocking is not an output-device failure');
    HTMLMediaElement.prototype.play = originalPlay;
    button('Test microphone').click(); await wait(100); await cleanup();
    assert(recorders.every(r => r.state === 'inactive'), 'unmount must stop every recorder');
    return 'PASS automatic limit, output denial/recovery, autoplay blocking, unmount cleanup';
  `),
  );
} catch (error) {
  console.error(browser("get", "text", "body"));
  throw error;
} finally {
  browser("close");
}
