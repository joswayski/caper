// Real Call/Chat components and voice client; synthetic audio and mocked HTTP,
// WebSocket and WebRTC. This is UI regression coverage, not live SFU validation.
// Run against Vite: node scripts/test-voice-controls.mjs [http://localhost:5174]
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const origin = new URL(process.argv[2] ?? 'http://localhost:5174');
assert.ok(['localhost', '127.0.0.1'].includes(origin.hostname), 'Use local Vite, never production');
const session = `voice-ui-${process.pid}`;
const artifacts = process.env.VOICE_TEST_ARTIFACTS && resolve(process.env.VOICE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
function browser(...args) {
  const result = JSON.parse(execFileSync('agent-browser', ['--session', session, '--args', '--autoplay-policy=no-user-gesture-required', ...args, '--json'], { encoding: 'utf8', timeout: 60000 }));
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = code => browser('eval', `(async () => { ${code} })()`).result;
const wait = code => browser('wait', '--fn', `Boolean(${code})`);
const click = label => browser('click', `[aria-label="${label}"]`);
const screenshot = name => { if (artifacts) browser('screenshot', '--full', `${artifacts}/${name}.png`); };
const centeredDialog = () => assert.ok(evaluate(`const r = document.querySelector('dialog[open]').getBoundingClientRect(); return Math.abs(r.left + r.width / 2 - innerWidth / 2) < 1 && Math.abs(r.top + r.height / 2 - innerHeight / 2) < 1;`), 'Dialog must stay centered within the viewport');

async function fixture() {
  const { default: React } = await import('/node_modules/.vite/deps/react.js');
  const { default: { createRoot } } = await import('/node_modules/.vite/deps/react-dom_client.js');
  // Use Call's exact module URL, including Vite's HMR cache key after edits.
  const source = await (await fetch('/src/pages/Call.tsx')).text();
  const { PublicCallClient } = await import(source.match(/from "(\/src\/media\/client\.ts[^"]*)"/)[1]);
  const { default: Call } = await import('/src/pages/Call.tsx');
  for (const element of document.body.children) element.hidden = true;
  const mount = document.createElement('div'); document.body.append(mount);
  const f = window.voiceFixture = { people: [], streams: new Set(), sockets: [], captures: [], devices: [], recorders: [], revision: 0 };
  const account = { id: 'fixture-user', username: 'fixture', displayName: 'UI fixture' };
  const author = { id: account.id, name: account.displayName, isGuest: false };
  const peer = { id: 'peer', name: 'Peach Donkey', isGuest: true };
  const snapshot = () => ({ participants: f.people, revision: f.revision });
  const frame = (event, data = {}) => new TextEncoder().encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (path === '/api/account/me') return Response.json(account);
    if (path === '/api/chat/session') return Response.json({ token: 'fixture-only', author });
    if (path === '/api/chat/general') return Response.json({
      space: { id: 'fixture', name: 'UI fixture' }, channel: { id: 'general', name: 'General' }, cursor: '1', hasMore: false,
      messages: [{ id: 'example', channelId: 'general', seq: '1', clientMessageId: 'fixture-message', author: peer, content: { version: 1, type: 'text', text: 'UI test fixture · synthetic audio, no live participants.' }, createdAt: '2026-09-21T16:00:00Z' }],
    });
    if (path.endsWith('/typing')) return new Response(null, { status: 204 });
    if (path === '/api/media/status') return Response.json({ enabled: true });
    if (path === '/api/media/presence' || path === '/api/media/snapshot') return Response.json(snapshot());
    if (path.startsWith('/api/media/') && path.endsWith('/events')) {
      let controller;
      return new Response(new ReadableStream({ start(next) {
        controller = next; f.streams.add(controller);
        next.enqueue(frame('ready')); next.enqueue(frame('snapshot', snapshot()));
        options.signal?.addEventListener('abort', () => { f.streams.delete(controller); controller.error(new DOMException('Cancelled', 'AbortError')); }, { once: true });
      }, cancel() { f.streams.delete(controller); } }), { headers: { 'content-type': 'text/event-stream' } });
    }
    if (path === '/api/media/join') {
      f.people = [{ id: 'self', name: account.displayName, muted: false, deafened: false, tracks: [] }]; f.revision++;
      return Response.json({ token: 'fixture-voice', id: 'self', iceServers: [] });
    }
    if (path === '/api/media/publish') return Response.json({ sessionDescription: { type: 'answer', sdp: 'v=0' } });
    if (['/api/media/state', '/api/media/leave', '/api/media/close'].includes(path)) return new Response(null, { status: 204 });
    if (path.startsWith('/api/')) throw Error(`Unexpected fixture request: ${path}`);
    return originalFetch(input, options);
  };
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    constructor(url, protocols) {
      super();
      if (!String(url).includes('/api/chat/events')) return new NativeSocket(url, protocols);
      f.sockets.push(this); setTimeout(() => this.frame({ type: 'ready', cursor: '1' }), 0);
    }
    frame(event) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(event) })); }
    close() { f.sockets = f.sockets.filter(socket => socket !== this); }
  };
  f.typing = () => f.sockets.forEach(socket => socket.frame({ type: 'typing.updated', channelId: 'general', author: peer, typing: true, revision: String(Date.now() * 1000) }));
  const context = new AudioContext(); await context.resume();
  navigator.mediaDevices.enumerateDevices = async () => [
    { deviceId: 'default', kind: 'audioinput', label: 'Desk microphone' },
    { deviceId: 'headset', kind: 'audioinput', label: 'Headset microphone' },
    { deviceId: 'default', kind: 'audiooutput', label: 'Speakers' },
    { deviceId: 'headphones', kind: 'audiooutput', label: 'Headphones' },
  ];
  PublicCallClient.prototype.prepareMicrophone = () => {};
  PublicCallClient.prototype.openMicrophone = async function (deviceId) {
    f.devices.push(deviceId); f.client = this;
    const track = context.createMediaStreamDestination().stream.getAudioTracks()[0];
    f.captures.push(track); return track;
  };
  window.RTCPeerConnection = class extends EventTarget {
    connectionState = 'connected'; iceGatheringState = 'complete'; senders = [];
    addTransceiver(track) { const sender = { track, async replaceTrack(next) { this.track = next; } }; this.senders.push(sender); return { mid: '0', sender }; }
    async createOffer() { return { type: 'offer', sdp: 'v=0' }; }
    async setLocalDescription(value) { this.localDescription = { toJSON: () => value }; }
    async setRemoteDescription() {}
    getSenders() { return this.senders; }
    getReceivers() { return []; }
    async getStats() { return new Map(); }
    close() { this.connectionState = 'closed'; }
  };
  const Recorder = window.MediaRecorder;
  window.MediaRecorder = class extends Recorder { constructor(...args) { super(...args); f.recorders.push(this); } };
  localStorage.removeItem('caper.chat.session');
  const root = createRoot(mount); root.render(React.createElement(Call));
  f.cleanup = async () => { root.unmount(); f.captures.forEach(track => track.stop()); await context.close(); };
}

try {
  browser('open', origin.href);
  browser('set', 'viewport', '1280', '900', '2');
  evaluate(`await (${fixture.toString()})();`);
  wait(`document.querySelector('#chat-message:not(:disabled)') && document.querySelector('.voice-button[aria-disabled="false"]')`);
  const bounds = () => evaluate(`return ['.chat-heading', '.chat-composer'].map(selector => { const r = document.querySelector(selector).getBoundingClientRect(); return [r.top, r.height]; });`);
  const initialBounds = bounds();
  assert.equal(evaluate(`return document.querySelector('.account-name').textContent;`), 'UI fixture', 'Profile must use display name, not username');
  assert.equal(evaluate(`return document.querySelector('.call-page').textContent.includes('Talk here, or keep typing');`), false);
  click('Input options');
  assert.equal(evaluate(`return !!document.querySelector('details[open] select, details[open] .volume-control');`), false, 'Device menu must have direct choices, no nested select or gain slider');
  browser('check', 'input[name="input-device"][value="headset"]');
  assert.equal(evaluate(`return document.querySelector('input[name="input-device"]:checked').value;`), 'headset');
  screenshot('voice-input-options');
  click('Output options');
  assert.equal(evaluate(`return document.querySelector('summary[aria-label="Input options"]').parentElement.open;`), false);
  browser('check', 'input[name="output-device"][value="headphones"]');
  assert.equal(evaluate(`return document.querySelector('input[name="output-device"]:checked').value;`), 'headphones');
  screenshot('voice-output-options');
  browser('press', 'Escape');
  assert.equal(evaluate(`return document.querySelector('summary[aria-label="Output options"]').parentElement.open;`), false);
  click('Settings');
  assert.equal(evaluate(`return !!document.querySelector('details[open] [aria-label="My voice level"]');`), true);
  browser('focus', '[aria-label="My voice level"]');
  browser('press', 'ArrowRight');
  assert.equal(evaluate(`return document.querySelector('.volume-control output').textContent;`), '101%');
  screenshot('voice-settings');
  browser('press', 'Escape');

  click('Join voice');
  wait(`document.querySelector('[aria-label="Leave voice"]')`);
  assert.deepEqual(bounds(), initialBounds, 'Joining must not move the chat header or composer');
  assert.equal(evaluate(`return voiceFixture.devices.at(-1);`), 'headset');
  click('Mute microphone'); wait(`document.querySelector('[aria-label="Unmute microphone"]')`);
  click('Deafen audio'); wait(`document.querySelector('[aria-label="Undeafen audio"]')`);
  evaluate(`voiceFixture.client.diagnostics = { join: 'Fixture join', microphoneSessionMs: 10, signalingMs: 20, transportMs: 30, rosterMs: 40, receivedBytes: 10000, receiveBitrate: 32000, sentBytes: 10000, sendBitrate: 32000, packetsLost: 0, maxJitterMs: 1, roundTripMs: 12, route: 'direct' }; voiceFixture.client.emit();`);
  click('Settings'); browser('find', 'role', 'button', 'click', '--name', 'Connection details', '--exact');
  wait(`document.querySelector('dialog[open] .call-diagnostics')`);
  assert.deepEqual(bounds(), initialBounds, 'Diagnostics must overlay rather than reflow messages');
  centeredDialog();
  screenshot('voice-connection-details');
  browser('press', 'Escape');
  wait(`!document.querySelector('dialog[open]')`);
  assert.equal(evaluate(`return document.activeElement.getAttribute('aria-label');`), 'Settings', 'Dialog close should restore focus to its settings trigger');

  click('Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] .mic-test-button')`);
  assert.deepEqual(bounds(), initialBounds, 'Mic test must not reflow messages');
  browser('click', '.mic-test-button');
  wait(`document.querySelector('.recording-clock')`);
  centeredDialog();
  screenshot('voice-mic-test');
  click('Close audio settings'); wait(`!document.querySelector('dialog[open]')`);
  assert.equal(evaluate(`return voiceFixture.recorders.at(-1).state;`), 'inactive', 'Closing during recording must cancel it');
  assert.equal(evaluate(`return voiceFixture.client.monitoring;`), false);
  assert.equal(evaluate(`return voiceFixture.client.muted && voiceFixture.client.deafened;`), true, 'Closing restores the previous mute/deafen intent');
  assert.equal(evaluate(`return voiceFixture.captures.at(-1).readyState;`), 'live', 'Connected mic test borrows the track; closing must not end the call');
  assert.deepEqual(bounds(), initialBounds);

  evaluate(`voiceFixture.typing();`);
  wait(`document.querySelectorAll('.chat-typing-dots i').length === 3`);
  const motion = evaluate(`const dots = [...document.querySelectorAll('.chat-typing-dots i')]; const sample = () => dots.map(dot => getComputedStyle(dot).transform); const before = sample(); await new Promise(r => setTimeout(r, 250)); return { before, after: sample(), delays: dots.map(dot => getComputedStyle(dot).animationDelay) };`);
  assert.notDeepEqual(motion.before, motion.after, 'Typing dots must actually move');
  assert.equal(new Set(motion.delays).size, 3, 'Typing dots must be staggered');
  screenshot('voice-controls-desktop');
  if (artifacts) {
    browser('record', 'start', `${artifacts}/typing-dots.webm`);
    evaluate(`voiceFixture.typing(); await new Promise(r => setTimeout(r, 1800));`);
    browser('record', 'stop');
  }
  browser('set', 'media', 'reduced-motion');
  assert.equal(evaluate(`return getComputedStyle(document.querySelector('.chat-typing-dots i')).animationName;`), 'none');
  browser('set', 'media', 'no-preference');

  browser('set', 'viewport', '390', '844', '2');
  click('Input options');
  assert.ok(evaluate(`const r = document.querySelector('details[open] .call-settings-panel').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth;`));
  screenshot('voice-controls-narrow');
  browser('press', 'Escape');
  click('Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] .mic-test-button')`);
  centeredDialog();
  screenshot('voice-mic-test-narrow');
  browser('press', 'Escape'); wait(`!document.querySelector('dialog[open]')`);
  click('Leave voice'); wait(`document.querySelector('[aria-label="Join voice"]')`);
  click('Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] .mic-test-button')`);
  click('Close audio settings'); wait(`!document.querySelector('dialog[open]')`);
  assert.equal(evaluate(`return voiceFixture.captures.at(-1).readyState;`), 'ended', 'Closing a pre-join test must release its microphone');
  assert.equal(evaluate(`return !!document.querySelector('.call-controls') || /huddle/i.test(document.querySelector('.call-page').textContent);`), false);
  evaluate(`await voiceFixture.cleanup();`);
  console.log('PASS device menus, selection, mute/deafen, stable chat geometry, diagnostics, recording cancellation, focus restoration, animated/reduced-motion typing dots, and narrow layout');
} catch (error) {
  console.error(evaluate(`return { alerts: [...document.querySelectorAll('[role="alert"]')].map(node => node.textContent), phase: window.voiceFixture?.client?.phase };`));
  throw error;
} finally {
  browser('close');
}
