// Real Chromium loopback, not Cloudflare or physical microphone validation.
// Generate synthetic speech with espeak-ng, then run against local Vite:
// node scripts/test-opus-dtx.mjs http://localhost:5173 /tmp/speech.wav
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const origin = new URL(process.argv[2] ?? "http://localhost:5173");
if (!["localhost", "127.0.0.1"].includes(origin.hostname)) throw new Error("Use local Vite, not production.");
if (!process.argv[3]) throw new Error("Supply a synthetic speech WAV, not a private microphone recording.");
const speech = readFileSync(process.argv[3]).toString("base64");
const session = `opus-dtx-${process.pid}`;
function browser(...args) {
  const input = args[0] === "eval" ? args.pop() : undefined;
  if (input) args.push("--stdin");
  return execFileSync("agent-browser", ["--session", session,
    "--args", "--autoplay-policy=no-user-gesture-required", ...args], { input, encoding: "utf8", timeout: 60_000 });
}

try {
  // A plain source document avoids running the app's animations or account flows.
  browser("open", new URL("/src/media/rtc.ts", origin).href);
  console.log(browser("eval", `(async () => {
    const { withOpusDtx, preferOpus } = await import('/src/media/rtc.ts');
    const assert = (condition, message) => { if (!condition) throw new Error(message); };
    const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
    const context = new AudioContext({sampleRate: 48000});
    const peers = [];
    const tracks = [];
    const sinks = [];
    try {
      await context.resume();
      const bytes = Uint8Array.from(atob(${JSON.stringify(speech)}), c => c.charCodeAt(0));
      const audio = await context.decodeAudioData(bytes.buffer);
      assert(audio.duration > 2 && audio.duration < 10, 'Use 2–10 seconds of synthetic speech');
      const gain = context.createGain();
      const destination = context.createMediaStreamDestination();
      gain.connect(destination);
      // Keep rendering actual zero-valued samples between phrases rather than
      // letting an exhausted source graph stop supplying audio to both encoders.
      const silenceSource = context.createConstantSource(); silenceSource.offset.value = 0;
      silenceSource.connect(gain); silenceSource.start();
      const track = destination.stream.getAudioTracks()[0];
      tracks.push(track);
      const sink = context.createGain(); sink.gain.value = 0; sink.connect(context.destination);
      async function pair(dtx) {
        const tx = new RTCPeerConnection({iceServers: []});
        const rx = new RTCPeerConnection({iceServers: []});
        peers.push(tx, rx);
        // Queue candidates until their corresponding remote description is installed.
        const toTx = [], toRx = [];
        tx.onicecandidate = e => { if (e.candidate) toRx.push(e.candidate); };
        rx.onicecandidate = e => { if (e.candidate) toTx.push(e.candidate); };
        const analyser = context.createAnalyser(); analyser.fftSize = 256;
        rx.ontrack = e => {
          tracks.push(e.track);
          const stream = new MediaStream([e.track]);
          // Chromium needs an HTML sink to drain its remote jitter buffer.
          const element = new Audio(); element.muted = true; element.srcObject = stream;
          sinks.push(element); void element.play();
          context.createMediaStreamSource(stream).connect(analyser).connect(sink);
        };
        preferOpus(tx.addTransceiver(track, {direction: 'sendonly', streams: [destination.stream]}));
        await tx.setLocalDescription(await tx.createOffer());
        await rx.setRemoteDescription(tx.localDescription);
        await rx.setLocalDescription(await rx.createAnswer());
        await tx.setRemoteDescription(dtx ? withOpusDtx(rx.localDescription.toJSON()) : rx.localDescription);
        for (let i = 0; i < 500; i++) {
          while (toRx.length) await rx.addIceCandidate(toRx.shift());
          while (toTx.length) await tx.addIceCandidate(toTx.shift());
          if (tx.connectionState === 'connected' && rx.connectionState === 'connected') break;
          await wait(10);
        }
        assert(tx.connectionState === 'connected' && rx.connectionState === 'connected', 'Loopback must connect');
        const opus = tx.getSenders()[0].getParameters().codecs.find(c => c.mimeType.toLowerCase() === 'audio/opus');
        assert(!!opus && /usedtx=1/.test(opus.sdpFmtpLine ?? '') === dtx, 'Sender must actually negotiate the requested DTX');
        return {tx, rx, analyser};
      }
      const baseline = await pair(false), dtx = await pair(true);
      async function stats(pair) {
        const outbound = [...(await pair.tx.getStats()).values()].find(s => s.type === 'outbound-rtp' && s.kind === 'audio');
        return {packets: outbound.packetsSent, bytes: outbound.bytesSent};
      }
      const play = volume => {
        gain.gain.value = volume;
        const source = context.createBufferSource(); source.buffer = audio;
        source.connect(gain); source.start(context.currentTime + 0.1);
      };
      play(0.3);
      await wait((audio.duration + 0.3) * 1000);
      await wait(2000); // Let codec hangover and queued audio finish before measuring silence.
      const before = await Promise.all([stats(baseline), stats(dtx)]);
      await wait(3000);
      const after = await Promise.all([stats(baseline), stats(dtx)]);
      const silence = after.map((value, i) => ({packets: value.packets - before[i].packets, bytes: value.bytes - before[i].bytes}));
      assert(silence[0].packets > 100, 'Continuous-transmission control must be active: ' + JSON.stringify(silence));
      assert(silence[1].packets < silence[0].packets * 0.3, 'DTX must materially reduce silence packet traffic');
      const samples = new Float32Array(256);
      const onset = [null, null];
      const energy = [0, 0];
      play(0.01); // Quiet synthetic speech, 30 dB below the preceding phrase.
      const started = performance.now();
      while (performance.now() - started < (audio.duration + 0.5) * 1000) {
        [baseline, dtx].forEach((pair, i) => {
          pair.analyser.getFloatTimeDomainData(samples);
          energy[i] += samples.reduce((sum, value) => sum + value * value, 0);
          if (onset[i] === null && samples.some(v => Math.abs(v) > 0.0001)) onset[i] = performance.now() - started;
        });
        await wait(10);
      }
      assert(onset.every(value => value !== null), 'Both receivers must decode quiet speech after silence: ' + JSON.stringify({onset, energy, silence}));
      assert(onset[1] - onset[0] < 80, 'DTX must not add an 80 ms onset delay relative to the control');
      assert(energy[0] > 0 && energy[1] / energy[0] > 0.8, 'DTX must retain quiet-phrase energy relative to control: ' + JSON.stringify({onset, energy, silence}));
      assert(track.enabled && track.readyState === 'live', 'Silence suppression must not mute or stop the track');
      return {result: 'PASS', silence: {baseline: silence[0], dtx: silence[1]}, quietSpeech: {onsetMs: onset, sampledEnergyRatio: energy[1] / energy[0]}, scope: 'Synthetic speech, local Chromium Opus; not physical listening or SFU validation'};
    } finally {
      sinks.forEach(element => { element.pause(); element.srcObject = null; });
      peers.forEach(peer => peer.close()); tracks.forEach(track => track.stop()); await context.close();
    }
  })()`));
} finally {
  browser("close");
}
