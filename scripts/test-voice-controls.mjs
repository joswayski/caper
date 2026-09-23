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
  const f = window.voiceFixture = { people: [], sockets: [], captures: [], devices: [], recorders: [], revision: 0 };
  const account = { id: 'fixture-user', username: 'fixture', displayName: 'UI fixture' };
  const author = { id: account.id, name: account.displayName, isGuest: false };
  const peer = { id: 'peer', name: 'Peach Donkey', isGuest: true };
  const snapshot = () => ({ participants: f.people, revision: f.revision });
  const originalFetch = window.fetch.bind(window);
  const respond = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (path === '/api/account/me') return Response.json(account);
    if (path === '/api/account/profile') {
      const updated = JSON.parse(options.body);
      if (updated.username === 'taken') return Response.json({ error: 'Taken' }, { status: 409 });
      Object.assign(account, updated); author.name = updated.displayName;
      return Response.json(account);
    }
    if (path === '/api/chat/session') return Response.json({ token: 'fixture-only', author });
    if (path === '/api/chat/general') return Response.json({
      space: { id: 'fixture', name: 'UI fixture' }, channel: { id: 'general', name: 'General' }, cursor: '1', hasMore: false,
      messages: [{ id: 'example', channelId: 'general', seq: '1', clientMessageId: 'fixture-message', author: peer, content: { version: 1, type: 'text', text: 'UI test fixture · synthetic audio, no live participants.' }, createdAt: '2026-09-21T16:00:00Z' }],
    });
    if (path.endsWith('/typing')) return new Response(null, { status: 204 });
    if (path === '/api/media/status') return Response.json({ enabled: true });
    if (path === '/api/media/presence' || path === '/api/media/snapshot') return Response.json(snapshot());
    if (path === '/api/media/join') {
      f.people = [{ id: 'self', name: account.displayName, muted: false, deafened: false, tracks: [] }]; f.revision++;
      return Response.json({ token: 'fixture-voice', id: 'self', iceServers: [] });
    }
    if (path === '/api/media/publish') return Response.json({ sessionDescription: { type: 'answer', sdp: 'v=0' } });
    if (['/api/media/state', '/api/media/leave', '/api/media/close'].includes(path)) return new Response(null, { status: 204 });
    if (path.startsWith('/api/')) throw Error(`Unexpected fixture request: ${path}`);
    return originalFetch(input, options);
  };
  window.fetch = (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if ((path.startsWith('/api/media/') && path !== '/api/media/status') || path.endsWith('/typing')) {
      throw Error(`Ephemeral operation bypassed gateway: ${path}`);
    }
    return respond(input, options);
  };
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    subscriptions = new Map();
    constructor(url, protocols) {
      super();
      if (!String(url).includes('/api/chat/events')) return new NativeSocket(url, protocols);
      f.sockets.push(this);
      setTimeout(() => this.frame({ type: 'hello', idleTimeoutSeconds: 600, serverTime: Date.now() }), 0);
    }
    async send(data) {
      const request = JSON.parse(data);
      if (request.type === 'heartbeat') this.frame({ type: 'heartbeat' });
      if (request.type === 'unsubscribe') this.subscriptions.delete(request.id);
      if (request.type === 'subscribe') {
        this.subscriptions.set(request.id, request);
        const event = request.kind === 'chat' ? { type: 'ready', cursor: request.after } : {
          type: 'snapshot', ...snapshot(),
          participants: request.token ? f.people : f.people.map(({ tracks, ...person }) => person),
        };
        this.frame({ type: 'event', id: request.id, event });
        this.frame({ type: 'subscribed', id: request.id });
      }
      if (request.type === 'command') {
        // Reuse fixture responses, without issuing network requests. The real
        // client must reach them through a command on this shared socket.
        const path = request.method === 'typing' ? `/api/chat/channels/${request.channelId}/typing`
          : `/api/media/${request.method.slice('media.'.length)}`;
        const response = await respond(path, { body: JSON.stringify(request.body) });
        this.frame({ type: 'result', id: request.id, status: response.status,
          body: response.status === 204 ? null : await response.json() });
      }
    }
    frame(event) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(event) })); }
    close() { f.sockets = f.sockets.filter(socket => socket !== this); }
  };
  f.typing = () => f.sockets.forEach(socket => {
    for (const [id, subscription] of socket.subscriptions) if (subscription.kind === 'chat') {
      socket.frame({ type: 'event', id, event: { type: 'typing.updated', channelId: 'general', author: peer, typing: true, revision: String(Date.now() * 1000) } });
    }
  });
  const context = new AudioContext(); await context.resume();
  const signal = context.createOscillator(); signal.start();
  // Device IDs are synthetic; exercise selection without requiring host hardware.
  HTMLMediaElement.prototype.setSinkId = async function () {};
  AudioContext.prototype.setSinkId = async function () {};
  f.sampleGains = [];
  const createMediaElementSource = AudioContext.prototype.createMediaElementSource;
  AudioContext.prototype.createMediaElementSource = function (element) {
    const source = createMediaElementSource.call(this, element);
    const connect = source.connect.bind(source);
    source.connect = node => { f.sampleGains.push(node); return connect(node); };
    return source;
  };
  f.addRemoteAudio = () => {
    const createGain = AudioContext.prototype.createGain;
    AudioContext.prototype.createGain = function () {
      f.outputGain = createGain.call(this);
      return f.outputGain;
    };
    f.people.push({ id: peer.id, name: peer.name, muted: false, deafened: false, tracks: [] });
    f.client.participants = [...f.people];
    f.remoteStream = context.createMediaStreamDestination().stream;
    f.client.remoteMedia.set('remote', { trackId: 'remote', participantId: peer.id, kind: 'microphone', stream: f.remoteStream });
    f.client.emit();
    requestAnimationFrame(() => requestAnimationFrame(() => { AudioContext.prototype.createGain = createGain; }));
  };
  navigator.mediaDevices.enumerateDevices = async () => [
    { deviceId: 'default', kind: 'audioinput', label: 'Desk microphone' },
    { deviceId: 'headset', kind: 'audioinput', label: 'Headset microphone' },
    { deviceId: 'default', kind: 'audiooutput', label: 'Speakers' },
    { deviceId: 'headphones', kind: 'audiooutput', label: 'Headphones' },
  ];
  PublicCallClient.prototype.prepareMicrophone = () => {};
  PublicCallClient.prototype.openMicrophone = async function (deviceId) {
    f.devices.push(deviceId); f.client = this;
    if (f.captureError) throw new DOMException('Fixture capture error', f.captureError);
    if (f.holdCapture) await new Promise(resolve => { f.releaseCapture = resolve; });
    const destination = context.createMediaStreamDestination();
    signal.connect(destination);
    const track = destination.stream.getAudioTracks()[0];
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
  f.cleanup = async () => { root.unmount(); f.captures.forEach(track => track.stop()); f.remoteStream?.getTracks().forEach(track => track.stop()); await context.close(); };
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
  assert.ok(evaluate(`const a = document.querySelector('.call-account').getBoundingClientRect(), b = document.querySelector('#chat-message').getBoundingClientRect(); return a.top === b.top && a.bottom === b.bottom;`), 'Account bar and single-line composer must align');
  assert.equal(evaluate(`return !!document.querySelector('.chat-composer button[type="submit"]');`), false);
  browser('click', '.account-profile');
  wait(`document.querySelector('.profile-dialog[open] input')`);
  browser('fill', '#username', 'taken');
  browser('click', '.profile-dialog button[type="submit"]');
  wait(`document.querySelector('.profile-dialog [role="alert"]')`);
  assert.equal(evaluate(`return document.querySelector('.profile-dialog [role="alert"]').textContent;`), 'That username is already taken.');
  browser('fill', '#username', 'updated_fixture');
  browser('fill', '#display-name', 'Updated fixture');
  browser('click', '.profile-dialog button[type="submit"]');
  wait(`!document.querySelector('dialog[open]')`);
  assert.equal(evaluate(`return document.querySelector('.account-name').textContent;`), 'Updated fixture');
  assert.equal(evaluate(`return document.activeElement.className;`), 'account-profile');
  browser('click', '.account-profile');
  wait(`document.querySelector('.profile-dialog[open] input')`);
  assert.equal(evaluate(`return document.querySelector('#username').value;`), 'updated_fixture');
  browser('press', 'Escape');
  wait(`!document.querySelector('dialog[open]')`);
  click('Input options');
  assert.equal(evaluate(`return !!document.querySelector('details[open] select');`), false, 'Device menu must have direct choices');
  browser('focus', '[aria-label="Input volume"]'); browser('press', 'End');
  assert.equal(evaluate(`return document.querySelector('[aria-label="Input volume"]').getAttribute('aria-valuenow');`), '200');
  browser('press', 'Home');
  assert.equal(evaluate(`return document.querySelector('[aria-label="Input volume"]').getAttribute('aria-valuenow');`), '0');
  browser('press', 'End');
  wait(`getComputedStyle(document.querySelector('summary[aria-label="Input options"] svg')).transform === 'matrix(-1, 0, 0, -1, 0, 0)'`);
  assert.equal(evaluate(`return getComputedStyle(document.querySelector('summary[aria-label="Input options"] svg')).transform;`), 'matrix(-1, 0, 0, -1, 0, 0)', 'Open chevron must point up');
  browser('check', 'input[name="input-device"][value="headset"]');
  assert.equal(evaluate(`return document.querySelector('input[name="input-device"]:checked').value;`), 'headset');
  screenshot('voice-input-options');
  click('Output options');
  assert.equal(evaluate(`return document.querySelector('summary[aria-label="Input options"]').parentElement.open;`), false);
  browser('check', 'input[name="output-device"][value="headphones"]');
  assert.equal(evaluate(`return document.querySelector('input[name="output-device"]:checked').value;`), 'headphones');
  browser('focus', '[aria-label="Output volume"]'); browser('press', 'PageDown');
  assert.equal(evaluate(`return document.querySelector('details[open] .output-volume output').textContent;`), '90%');
  screenshot('voice-output-options');
  browser('press', 'Escape');
  assert.equal(evaluate(`return document.querySelector('summary[aria-label="Output options"]').parentElement.open;`), false);
  click('Settings');
  assert.equal(evaluate(`return !!document.querySelector('details[open] [role="slider"]');`), false);
  assert.equal(evaluate(`return document.querySelector('details[open]').textContent.includes('@fixture');`), false);
  assert.equal(evaluate(`return !!document.querySelector('details[open] a[href="/profile"]');`), false);
  assert.ok(evaluate(`return [...document.querySelectorAll('details[open] button')].every(button => { const style = getComputedStyle(button); return style.borderTopWidth === '0px' && style.backgroundColor === 'rgba(0, 0, 0, 0)'; });`), 'Menu actions must not look like bordered inputs');
  screenshot('voice-settings');
  browser('press', 'Escape');

  click('Join voice');
  wait(`document.querySelector('[aria-label="Leave voice"]')`);
  assert.equal(evaluate(`return voiceFixture.sockets.length;`), 1, 'Chat and joined voice must share one socket');
  assert.deepEqual(bounds(), initialBounds, 'Joining must not move the chat header or composer');
  assert.equal(evaluate(`return voiceFixture.devices.at(-1);`), 'headset');
  evaluate(`voiceFixture.addRemoteAudio();`);
  wait(`voiceFixture.outputGain`);
  const gain = () => evaluate(`return voiceFixture.outputGain.gain.value;`);
  assert.ok(Math.abs(gain() - 0.9) < 0.00001, 'Master volume must reach the audio graph');
  click('Audio controls for Peach Donkey');
  browser('focus', '[aria-label="Peach Donkey volume"]'); browser('press', 'PageUp');
  wait(`document.querySelector('[aria-label="Peach Donkey volume"]').getAttribute('aria-valuenow') === '110' && Math.abs(voiceFixture.outputGain.gain.value - 0.99) < 0.00001`);
  assert.ok(Math.abs(gain() - 0.99) < 0.00001, '90% master × 110% participant must produce 99% gain');
  click('Audio controls for Peach Donkey');
  click('Output options'); browser('focus', '[aria-label="Output volume"]'); browser('press', 'Home');
  wait(`voiceFixture.outputGain.gain.value === 0`);
  assert.equal(gain(), 0, 'Zero master volume must silence playback');
  browser('press', 'End');
  wait(`Math.abs(voiceFixture.outputGain.gain.value - 2.2) < 0.00001`);
  assert.ok(Math.abs(gain() - 2.2) < 0.00001, '200% master × 110% participant must produce 220% gain');
  browser('press', 'PageDown'); browser('press', 'Escape');
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
  click('Output options');
  assert.ok(evaluate(`const r = document.querySelector('details[open] .call-settings-panel').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth;`));
  screenshot('voice-output-narrow');
  browser('press', 'Escape');
  click('Settings'); screenshot('voice-settings-narrow'); browser('press', 'Escape');
  click('Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] .mic-test-button')`);
  centeredDialog();
  screenshot('voice-mic-test-narrow');
  browser('click', '.mic-test-button');
  wait(`document.querySelector('.recording-clock')`);
  evaluate(`await new Promise(resolve => setTimeout(resolve, 1000));`);
  browser('click', '.mic-test-button');
  wait(`document.querySelector('audio[aria-label="Natural microphone sample"]')`);
  wait(`voiceFixture.sampleGains.length > 0`);
  assert.ok(evaluate(`return voiceFixture.sampleGains.every(node => Math.abs(node.gain.value - 1.9) < 0.00001);`), 'Mic-test playback must amplify to 190% without setting native volume above one');
  browser('press', 'Escape'); wait(`!document.querySelector('dialog[open]')`);
  wait(`voiceFixture.sampleGains.every(node => node.context.state === 'closed')`);
  click('Leave voice'); wait(`document.querySelector('[aria-label="Join voice"]')`);
  click('Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] .mic-test-button')`);
  click('Close audio settings'); wait(`!document.querySelector('dialog[open]')`);
  assert.equal(evaluate(`return voiceFixture.captures.at(-1).readyState;`), 'ended', 'Closing a pre-join test must release its microphone');
  evaluate(`voiceFixture.holdCapture = true;`);
  click('Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] [role="status"]')`);
  click('Close audio settings');
  evaluate(`voiceFixture.holdCapture = false; voiceFixture.captureError = 'NotFoundError';`);
  click('Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] [role="alert"]')`);
  assert.ok(evaluate(`return document.querySelector('dialog[open]').textContent.includes('Connect a microphone');`));
  assert.equal(evaluate(`return !!document.querySelector('dialog[open] [role="status"]');`), false);
  evaluate(`voiceFixture.captureError = undefined;`);
  browser('find', 'role', 'button', 'click', '--name', 'Try again', '--exact');
  wait(`document.querySelector('dialog[open] .mic-test-button')`);
  evaluate(`voiceFixture.releaseCapture();`);
  wait(`voiceFixture.captures.at(-1).readyState === 'ended'`);
  assert.equal(evaluate(`return !!document.querySelector('dialog[open] .mic-test-button');`), true, 'Late cancelled capture must not disturb the successful retry');
  click('Close audio settings');
  assert.equal(evaluate(`return !!document.querySelector('.call-controls') || /huddle/i.test(document.querySelector('.call-page').textContent);`), false);
  evaluate(`await voiceFixture.cleanup();`);
  console.log('PASS profile modal save/conflict/focus, aligned composer without Send, 0–200% input/output, master/participant/sample gain and cleanup, device menus, mute/deafen, diagnostics, recording cancellation, reduced motion, and narrow layout');
} catch (error) {
  console.error(evaluate(`return { alerts: [...document.querySelectorAll('[role="alert"]')].map(node => node.textContent), phase: window.voiceFixture?.client?.phase };`));
  throw error;
} finally {
  browser('close');
}
