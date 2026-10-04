// Real Call/Chat components and voice client; synthetic audio and mocked HTTP,
// WebSocket and WebRTC. This is UI regression coverage, not live SFU validation.
// Run against Vite: node scripts/test-voice-controls.mjs [http://localhost:5174]
// Focus on stable channel rows with VOICE_TEST_CHANNEL_ROWS=1.
// Focus only on avatar borders with VOICE_TEST_AVATARS=1.
// Focus on the profile/voice dock with VOICE_TEST_DOCK=1.
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
  const { default: Spaces } = await import('/src/spaces/Spaces.tsx');
  // Remove the homepage's simulated channels from document-level test queries.
  for (const element of [...document.body.children]) element.remove();
  const mount = document.createElement('div'); document.body.append(mount);
  const f = window.voiceFixture = { people: [], sockets: [], captures: [], devices: [], recorders: [], commands: [], revision: 0 };
  const account = { id: 'fixture-user', username: 'fixture', displayName: 'UI fixture', avatarId: 0, debugEnabled: true };
  const space = { id: 'workspace123', name: 'Test space', ownerId: account.id };
  const channels = ['alpha', 'beta'].map(name => ({ id: name.padEnd(12, '0'), name, spaceId: space.id, private: false }));
  channels.push({ id: 'private00000', name: 'planning-for-the-next-release', spaceId: space.id, private: true });
  const author = { id: account.id, name: account.displayName, isGuest: false };
  const peer = { id: 'peer', name: 'Peach Donkey', isGuest: true };
  const snapshot = () => ({ participants: f.people, revision: f.revision });
  const originalFetch = window.fetch.bind(window);
  const respond = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (path === '/api/account/me') return Response.json(account);
    if (path === '/api/dms') return Response.json({ conversations: [] });
    if (path === '/api/spaces') return Response.json({ spaces: [space], limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 } });
    if (path === '/api/spaces/workspace123') return Response.json({ space, channels, members: [{ ...account, owner: true }] });
    if (path === '/api/spaces/workspace123/channels/private00000/members') return Response.json({ members: [account] });
    if (path.startsWith('/api/chat/channels/') && path.endsWith('/messages')) return Response.json({ space, channel: channels.find(channel => path.includes(`/${channel.id}/`)), messages: [], cursor: '0', hasMore: false });
    if (path.endsWith('/media/status')) return Response.json({ enabled: true });
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
      f.people = [{ id: 'self', name: account.displayName, avatarId: account.avatarId, muted: false, deafened: false, tracks: [] }]; f.revision++;
      return Response.json({ token: 'fixture-voice', id: 'self', iceServers: [] });
    }
    if (path === '/api/media/publish') return Response.json({ sessionDescription: { type: 'answer', sdp: 'v=0' } });
    if (path === '/api/media/warm') return Response.json({ error: 'Warm voice is not supported by this fixture' }, { status: 422 });
    if (['/api/media/state', '/api/media/leave', '/api/media/close', '/api/media/prepare'].includes(path)) return new Response(null, { status: 204 });
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
        const event = request.kind === 'chat' ? { type: 'ready', cursor: request.after } : request.kind === 'presence' ? { type: 'snapshot', members: request.userIds.map(userId => ({ userId, status: 'online' })) } : {
          type: 'snapshot', ...snapshot(),
          participants: request.token ? f.people : f.people.map(({ tracks, ...person }) => person),
        };
        this.frame({ type: 'event', id: request.id, event });
        this.frame({ type: 'subscribed', id: request.id });
      }
      if (request.type === 'command') {
        f.commands.push(request);
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
  f.publishPresence = people => {
    f.people = people; f.revision++;
    f.sockets.forEach(socket => {
      for (const [id, subscription] of socket.subscriptions) if (subscription.kind === 'media' && !subscription.token) {
        socket.frame({ type: 'event', id, event: { type: 'snapshot', ...snapshot(), participants: people.map(({ tracks, ...person }) => person) } });
      }
    });
  };
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
    configuration = {};
    getConfiguration() { return this.configuration; }
    setConfiguration(value) { this.configuration = value; }
    addTransceiver(track) { const sender = { track: typeof track === 'string' ? null : track, async replaceTrack(next) { this.track = next; } }; this.senders.push(sender); return { mid: '0', sender }; }
    async createOffer() { return { type: 'offer', sdp: 'v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\n' }; }
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
  // Isolate the mounted fixture from the hidden app's router. Spaces still uses
  // real browser history, but must not mount a second copy in the hidden app.
  history.pushState = History.prototype.pushState.bind(history);
  history.replaceState = History.prototype.replaceState.bind(history);
  const root = createRoot(mount); root.render(React.createElement(Call));
  f.showSpaces = (owner = true) => {
    space.ownerId = owner ? account.id : 'other-owner';
    history.replaceState({}, '', `/spaces?space=${space.id}&channel=${channels[0].id}`);
    root.render(React.createElement(Spaces, { key: owner ? 'owner' : 'member' }));
  };
  f.cleanup = async () => { root.unmount(); f.captures.forEach(track => track.stop()); f.remoteStream?.getTracks().forEach(track => track.stop()); await context.close(); };
}

try {
  browser('open', origin.href);
  browser('set', 'viewport', '1280', '900', '2');
  // Wait for Start's document-level hydration before adding a second root.
  // Otherwise hydration can replace the body and discard the fixture mount.
  browser('wait', '1500');
  evaluate(`await (${fixture.toString()})();`);
  wait(`document.querySelector('#chat-message:not(:disabled)') && document.querySelector('.voice-button[aria-disabled="false"]')`);
  if (process.env.VOICE_TEST_DOCK === '1') {
    evaluate(`voiceFixture.showSpaces();`);
    wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === 'alpha' && document.querySelector('.voice-button[aria-disabled="false"]')`);
    click('Mute microphone');
    click('Join voice');
    wait(`document.querySelector('[aria-label="Leave voice"]')`);
    for (const width of [1280, 390]) {
      browser('set', 'viewport', String(width), '900', '2');
      if (width === 390) browser('find', 'role', 'button', 'click', '--name', 'Browse', '--exact');
      evaluate(`await document.fonts.ready; await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`);
      assert.ok(evaluate(`const dock = document.querySelector('.call-account').getBoundingClientRect();
        return [...document.querySelectorAll('.call-account :is(.account-avatar, .account-name, .voice-icon-button, .call-settings-trigger)')].every(node => {
          const box = node.getBoundingClientRect();
          return Math.abs(box.top + box.height / 2 - (dock.top + dock.height / 2)) < .5 && box.top >= dock.top && box.bottom <= dock.bottom;
        });`), 'Connected profile/audio controls are centered and contained, not clipped');
      assert.equal(evaluate(`return document.querySelector('.voice-dock-channel').matches('button, a, [role="button"], [tabindex]');`), false, 'Connection status is not an interactive control');
      browser('hover', '.voice-dock-channel');
      assert.deepEqual(evaluate(`const status = document.querySelector('.voice-dock-channel'); return [getComputedStyle(status).backgroundColor, getComputedStyle(status).cursor, getComputedStyle(status.querySelector('small')).textDecorationLine];`), ['rgba(0, 0, 0, 0)', 'default', 'none'], 'Status hover has no highlight, hand cursor, or underline');
      const beforeClick = evaluate(`return location.href;`);
      browser('click', '.voice-dock-channel');
      assert.equal(evaluate(`return location.href;`), beforeClick, 'Status clicks do not navigate');
      assert.equal(evaluate(`return !!document.querySelector('dialog[open]');`), false, 'Status clicks do not open a dialog');
      assert.ok(evaluate(`return !!document.querySelector('[aria-label="Leave voice"]');`), 'Disconnect remains an explicit button');
      browser('mouse', 'move', '5', '5');
      screenshot(`dock-connected-${width}`);
    }
    click('Leave voice');
    wait(`!document.querySelector('.connected-channel')`);
    assert.equal(evaluate(`return voiceFixture.captures.at(-1).readyState;`), 'ended', 'Disconnect still releases the microphone');
    evaluate(`await voiceFixture.cleanup();`);
    console.log('PASS centered/unclipped connected profile dock, neutral non-interactive status, and working disconnect at 1280px/390px (mock signaling/WebRTC)');
  } else if (process.env.VOICE_TEST_AVATARS === '1') {
    evaluate(`voiceFixture.showSpaces();`);
    wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === 'alpha' && document.querySelector('.voice-button[aria-disabled="false"]')`);
    click('Mute microphone');
    click('Join voice');
    wait(`document.querySelector('[aria-label="Leave voice"]')`);
    evaluate(`voiceFixture.addRemoteAudio(); await document.fonts.ready;`);
    browser('click', '.channel-line:has(.channel-select[aria-current="page"]) .voice-stack');
    wait(`document.querySelectorAll('.voice-occupants .avatar.quiet').length === 2`);
    for (const width of [1280, 390]) {
      browser('set', 'viewport', String(width), '900', '2');
      if (width === 390) browser('find', 'role', 'button', 'click', '--name', 'Browse', '--exact');
      evaluate(`await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`);
      assert.deepEqual(evaluate(`return [...document.querySelectorAll('.voice-occupants .avatar')].map(el => { const s = getComputedStyle(el); return [s.borderTopColor, s.boxShadow, s.width, s.height]; });`), Array(2).fill(['rgba(0, 0, 0, 0)', 'none', '20px', '20px']), 'Idle image and initial avatars have no visible border or halo, without resizing');
      assert.deepEqual(evaluate(`return [...document.querySelectorAll('.voice-occupants .avatar > span')].map(el => el.getAttribute('data-avatar-id'));`), ['0', null], 'Exercise both image artwork and the initials fallback');
      screenshot(`voice-avatars-idle-${width}`);
      click('Unmute microphone');
      wait(`document.querySelector('.voice-occupants .avatar.speaking')`);
      assert.equal(evaluate(`return getComputedStyle(document.querySelector('.voice-occupants .avatar.speaking')).borderTopColor;`), 'rgb(99, 122, 67)', 'Speaking retains the green indicator');
      assert.notEqual(evaluate(`return getComputedStyle(document.querySelector('.voice-occupants .avatar.speaking')).boxShadow;`), 'none');
      screenshot(`voice-avatars-speaking-${width}`);
      click('Mute microphone');
      wait(`document.querySelectorAll('.voice-occupants .avatar.quiet').length === 2`);
    }
    evaluate(`await voiceFixture.cleanup();`);
    console.log('PASS borderless idle image/initial avatars and green speaking indicator at 1280px and 390px (mock signaling/WebRTC)');
  } else if (process.env.VOICE_TEST_CHANNEL_ROWS === '1') {
    evaluate(`voiceFixture.showSpaces();`);
    wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === 'alpha' && document.querySelector('.voice-button[aria-disabled="false"]')`);
    evaluate(`await document.fonts.ready;`);
    const geometry = () => evaluate(`return [...document.querySelectorAll('.channel-line')].map(line => [...line.querySelectorAll('.channel-select, .channel-manage, .channel-join, .channel-join-slot')].map(node => { const r = node.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; }));`);
    const countPosition = () => evaluate(`const r = document.querySelector('.channel-section-toggle .section-count').getBoundingClientRect(); return [r.x, r.y];`);
    const actionContentsFit = () => evaluate(`return [...document.querySelectorAll('.channel-join')].every(button => { const bounds = button.getBoundingClientRect(), icon = button.querySelector('svg').getBoundingClientRect(), label = button.querySelector('.channel-join-label').getBoundingClientRect(); return icon.width === 14 && icon.height === 14 && icon.left > bounds.left && icon.right < label.left && label.right < bounds.right && label.top >= bounds.top && label.bottom <= bounds.bottom; });`);
    const checkChannelTargets = () => {
      for (const area of ['leading padding', 'icon', 'name', 'trailing padding']) {
        for (const name of ['beta', 'alpha']) {
          const [x, y] = evaluate(`const button = [...document.querySelectorAll('.channel-select')].find(button => button.textContent === ${JSON.stringify(name)});
            const r = button.getBoundingClientRect(), area = ${JSON.stringify(area)};
            const target = area === 'icon' ? button.querySelector('svg') : area === 'name' ? button.querySelector('span') : button;
            const t = target.getBoundingClientRect();
            const x = area === 'leading padding' ? r.left + 3 : area === 'trailing padding' ? r.right - 3 : t.left + Math.min(t.width / 2, 20);
            if (!button.contains(document.elementFromPoint(x, r.top + r.height / 2))) throw Error('Channel target is obscured: ' + area);
            if (getComputedStyle(target).cursor !== 'pointer') throw Error('Channel target needs a hand cursor: ' + area);
            return [Math.round(x), Math.round(r.top + r.height / 2)];`);
          browser('mouse', 'move', String(x), String(y));
          browser('mouse', 'down', 'left'); browser('mouse', 'up', 'left');
          wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === ${JSON.stringify(name)} && document.querySelector('.chat-heading h2')?.textContent.includes(${JSON.stringify(name)})`);
        }
      }
      for (const selector of ['.space-rail button:enabled', '.call-account button:enabled', '.browse-channels']) {
        assert.ok(evaluate(`const buttons = [...document.querySelectorAll(${JSON.stringify(selector)})]; return buttons.length > 0 && buttons.every(button => getComputedStyle(button).cursor === 'pointer');`), `${selector}: enabled controls need hand cursors`);
      }
      assert.ok(evaluate(`return [...document.querySelectorAll('.channel-manage')].every(button => getComputedStyle(button).cursor === 'pointer');`), 'Owner channel options need hand cursors');
      assert.ok(evaluate(`const button = document.querySelector('.browse-channels');
        button.disabled = true; const disabled = getComputedStyle(button).cursor; button.disabled = false;
        button.setAttribute('aria-disabled', 'true'); const ariaDisabled = getComputedStyle(button).cursor; button.removeAttribute('aria-disabled');
        return disabled !== 'pointer' && ariaDisabled !== 'pointer';`), 'Disabled controls must not advertise a click');
    };
    for (const width of [1280, 390]) {
      browser('set', 'viewport', String(width), '900', '2');
      evaluate(`await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`);
      if (width === 390) browser('find', 'role', 'button', 'click', '--name', 'Browse', '--exact');
      browser('mouse', 'move', '5', '5');
      evaluate(`document.activeElement.blur();`);
      const joinsBeforeSettings = evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').length;`);
      const before = geometry();
      assert.equal(before.length, 3, 'Every channel, including empty/unselected/private channels, has voice actions');
      assert.ok(evaluate(`return [...document.querySelectorAll('.channel-line')].every(line => { const name = line.querySelector('.channel-select').getBoundingClientRect(), action = line.querySelector('.channel-join').getBoundingClientRect(); return action.top >= name.bottom && getComputedStyle(line.querySelector('.channel-manage')).opacity === '1'; });`), 'Voice actions must sit below the name; channel actions never depend on hover');
      assert.ok(evaluate(`return [...document.querySelectorAll('.channel-line')].every(line => { const name = line.querySelector('.channel-select').getBoundingClientRect(), action = line.querySelector('.channel-join').getBoundingClientRect(); return action.top - name.bottom <= 1 && (matchMedia('(hover: hover) and (pointer: fine) and (min-width: 761px)').matches ? name.height === 32 && action.height === 28 : name.height >= 44 && action.height >= 44); });`), 'Compact desktop rows have no extra gap; mobile/no-hover keeps 44px targets');
      assert.ok(evaluate(`return [...document.querySelectorAll('.channel-join')].every(button => button.querySelector('.lucide-speech')?.getAttribute('aria-hidden') === 'true');`), 'Join actions have a decorative speaking icon without changing their accessible names');
      assert.ok(actionContentsFit(), 'Speaking icons and Join labels must fit inside the fixed-width actions');
      assert.equal(evaluate(`return document.querySelectorAll('.voice-stack').length;`), 0, 'Empty channels do not show a zero-occupancy count');
      assert.ok(evaluate(`return [...document.querySelectorAll('.channel-voice')].every(row => row.innerText.trim() === 'Join voice');`), 'Empty channels retain Join without redundant no-one-in-voice text');
      assert.ok(evaluate(`return [...document.querySelectorAll('.channel-join')].every(button => { const style = getComputedStyle(button); return style.backgroundColor === 'rgba(0, 0, 0, 0)' && style.borderTopColor === 'rgba(0, 0, 0, 0)' && style.color === 'rgb(185, 188, 190)' && style.fontWeight === '600'; });`), 'Resting voice actions use muted neutral text without a colored fill or outline');
      screenshot(`channel-${width}-empty`);
      if (evaluate(`return matchMedia('(hover: hover)').matches;`)) {
        browser('hover', '[aria-label="Join voice"]');
        wait(`getComputedStyle(document.querySelector('[aria-label="Join voice"]')).color === 'rgb(243, 244, 245)'`);
        assert.ok(evaluate(`const style = getComputedStyle(document.querySelector('[aria-label="Join voice"]')); return style.backgroundColor === 'rgb(28, 31, 33)' && style.borderTopColor === 'rgb(52, 56, 59)' && style.color === 'rgb(243, 244, 245)';`), 'Hover gives neutral feedback, not a primary-action fill');
        assert.deepEqual(geometry(), before, 'Hover feedback cannot move actions');
        screenshot(`channel-${width}-hover`);
        browser('mouse', 'move', '5', '5');
      }
      browser('focus', '[aria-label="Manage alpha"]');
      browser('press', 'Tab');
      assert.equal(evaluate(`return document.activeElement.getAttribute('aria-label');`), 'Join voice', 'Join remains reachable immediately after channel settings');
      wait(`getComputedStyle(document.activeElement).color === 'rgb(243, 244, 245)'`);
      assert.ok(evaluate(`const button = document.activeElement, style = getComputedStyle(button); return button.matches(':focus-visible') && style.outlineStyle === 'solid' && style.outlineWidth === '2px' && style.color === 'rgb(243, 244, 245)';`), 'Keyboard focus retains a visible outline and readable action text');
      assert.deepEqual(geometry(), before, 'Keyboard feedback cannot move actions');
      screenshot(`channel-${width}-focus`);
      evaluate(`document.activeElement.blur();`);
      evaluate(`voiceFixture.publishPresence([{ id: 'peer', name: 'Mock peer', muted: false, deafened: false, tracks: [] }, { id: 'other', name: 'Other mock peer', muted: true, deafened: false, tracks: [] }]);`);
      wait(`document.querySelector('.voice-stack-count')?.textContent === '2 in voice'`);
      assert.deepEqual(geometry(), before, 'Roster arrivals cannot move names, menus or actions');
      assert.ok(evaluate(`return [...document.querySelectorAll('.voice-occupants-inner')].every(node => node.inert);`), 'Participant lists start collapsed');
      assert.ok(evaluate(`return [...document.querySelectorAll('.voice-stack-count')].every(node => node.scrollWidth <= node.clientWidth);`), 'Voice counts must remain readable, not ellipsized');
      browser('hover', '.channel-select[aria-current="page"]');
      browser('mouse', 'move', '5', '5');
      assert.deepEqual(geometry(), before, 'Hover cannot move any controls');
      click('Manage planning-for-the-next-release');
      assert.deepEqual(geometry(), before, 'Opening channel actions must overlay, not reflow the channel list');
      assert.ok(evaluate(`const menu = document.querySelector('.channel-menu[open]'); return menu.querySelector('.space-actions').getBoundingClientRect().top >= menu.closest('.channel-line').getBoundingClientRect().bottom;`), 'Channel actions must not cover their own Join button');
      screenshot(`channel-${width}-menu`);
      browser('press', 'Escape');
      assert.equal(evaluate(`return document.activeElement.getAttribute('aria-label');`), 'Manage planning-for-the-next-release');
      browser('press', 'Enter');
      browser('mouse', 'move', '5', '5'); browser('mouse', 'down', 'left'); browser('mouse', 'up', 'left');
      assert.equal(evaluate(`return !!document.querySelector('.channel-menu[open]');`), false, 'Outside clicks dismiss channel actions');
      click('Manage planning-for-the-next-release');
      browser('find', 'role', 'button', 'click', '--name', 'Channel settings', '--exact');
      wait(`document.querySelector('.space-dialog[open] .member-manager')`);
      assert.equal(evaluate(`return document.querySelector('.channel-privacy input').checked;`), true);
      browser('press', 'Escape');
      assert.equal(evaluate(`return document.activeElement.getAttribute('aria-label');`), 'Manage planning-for-the-next-release', 'Settings dismissal must restore focus to the visible channel action');
      assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').length;`), joinsBeforeSettings, 'Opening settings must not connect voice');
      screenshot(`channel-${width}-before-join`);
      evaluate(`
        voiceFixture.joinPositions = [];
        const button = document.querySelector('.channel-join[data-channel="alpha0000000"]');
        voiceFixture.actionNode = button;
        const row = button.closest('.channel-line');
        const start = performance.now();
        const sample = () => { const r = row.querySelector('.channel-join, .channel-join-slot').getBoundingClientRect(); voiceFixture.joinPositions.push([r.x, r.y, r.width, r.height]); if (performance.now() - start < 600) requestAnimationFrame(sample); };
        requestAnimationFrame(sample);
      `);
      browser('focus', '[aria-label="Join voice"]');
      browser('press', 'Enter');
      wait(`voiceFixture.client?.phase === 'connected' && document.querySelector('[aria-label="Leave voice"]')`);
      browser('wait', '650');
      assert.deepEqual(geometry(), before, 'Connecting removes the duplicate action without moving other targets');
      assert.equal(evaluate(`return new Set(voiceFixture.joinPositions.map(r => JSON.stringify(r))).size;`), 1, 'No intermediate animation may shift the Join target');
      assert.ok(evaluate(`return !voiceFixture.actionNode.isConnected && document.querySelectorAll('[aria-label="Leave voice"]').length === 1 && !document.querySelector('[aria-label="Leave voice in #alpha"]') && document.activeElement === document.querySelector('.voice-hangup');`), 'Disconnect exists only in the dock, which receives keyboard focus after joining');
      assert.ok(evaluate(`return !!document.querySelector('.voice-hangup .lucide-phone-off') && !!document.querySelector('[aria-label="Switch voice to #beta"] .lucide-speech');`), 'The dock uses the disconnect icon; switching retains the speaking icon');
      assert.ok(actionContentsFit(), 'Switch labels must fit with their icons');
      screenshot(`channel-${width}-joined`);
      if (width === 390) {
        click('Close navigation');
        // This fixture resized an existing desktop session with members open.
        click('Hide member list');
        assert.ok(evaluate(`const composer = document.querySelector('#chat-message'), r = composer.getBoundingClientRect(); return document.querySelector('.voice-hangup').getBoundingClientRect().height >= 44 && document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === composer;`), 'Narrow conversation retains the dock disconnect target and unobscured text entry');
        screenshot('channel-390-conversation');
        browser('find', 'role', 'button', 'click', '--name', 'Browse', '--exact');
      }
      browser('click', '.channel-line:has(.channel-select[aria-current="page"]) .voice-stack');
      wait(`document.querySelector('#voice-occupants-alpha0000000').getBoundingClientRect().height > 20`);
      assert.equal(evaluate(`return document.querySelector('#voice-occupants-alpha0000000 .voice-occupants-inner').inert;`), false);
      screenshot(`channel-${width}-expanded`);
      browser('click', '.channel-line:has(.channel-select[aria-current="page"]) .voice-stack');
      wait(`document.querySelector('#voice-occupants-alpha0000000').getBoundingClientRect().height === 0`);
      assert.deepEqual(geometry(), before, 'Collapsing the roster returns to the same row layout');
      evaluate(`voiceFixture.joinedTrack = voiceFixture.captures.at(-1); voiceFixture.leaveCount = voiceFixture.commands.filter(c => c.method === 'media.leave').length;`);
      browser('find', 'role', 'button', 'click', '--name', 'beta', '--exact');
      wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === 'beta'`);
      assert.equal(evaluate(`return voiceFixture.joinedTrack.readyState;`), 'live', 'Selecting text must not leave the current voice channel');
      assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`), evaluate(`return voiceFixture.leaveCount;`));
      if (width === 390) browser('find', 'role', 'button', 'click', '--name', 'Browse', '--exact');
      assert.equal(evaluate(`return document.querySelector('[aria-label="Switch voice to #beta"]').textContent;`), 'Switch here');
      click('Switch voice to #beta');
      wait(`voiceFixture.client?.phase === 'connected' && document.querySelector('.channel-line:has(.channel-select[aria-current="page"]) .channel-join-slot')`);
      assert.equal(evaluate(`return voiceFixture.joinedTrack.readyState;`), 'ended', 'Explicit switch must release the old capture');
      assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).channelId;`), 'beta00000000');
      assert.equal(evaluate(`return document.querySelectorAll('[aria-label="Leave voice"]').length;`), 1, 'Switching preserves only the dock disconnect');
      click('Leave voice');
      wait(`voiceFixture.client.phase === 'idle'`);
      browser('find', 'role', 'button', 'click', '--name', 'alpha', '--exact');
      wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === 'alpha'`);
      if (width === 390) browser('find', 'role', 'button', 'click', '--name', 'Browse', '--exact');
      evaluate(`voiceFixture.publishPresence([]); voiceFixture.holdCapture = true; voiceFixture.releaseCapture = undefined;`);
      click('Join voice');
      wait(`document.querySelector('[aria-label="Joining voice in #alpha"]') && voiceFixture.releaseCapture`);
      assert.deepEqual(geometry(), before, 'Pending microphone permission/capture must not shift controls');
      assert.equal(evaluate(`return document.querySelector('[aria-label="Joining voice in #alpha"]').getAttribute('aria-disabled');`), 'true');
      assert.ok(evaluate(`return !!document.querySelector('[aria-label="Joining voice in #alpha"] .lucide-speech');`), 'Joining retains the speaking icon');
      assert.ok(actionContentsFit(), 'Pending labels must fit with their icons');
      const joins = evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').length;`);
      evaluate(`document.querySelector('[aria-label="Joining voice in #alpha"]').click();`);
      assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').length;`), joins, 'Busy action must not issue a second Join');
      screenshot(`channel-${width}-joining`);
      wait(`document.querySelector('[aria-label="Cancel joining voice"]')`);
      click('Cancel joining voice');
      wait(`document.querySelector('[aria-label="Join voice"]')`);
      evaluate(`voiceFixture.holdCapture = false; voiceFixture.releaseCapture();`);
      wait(`voiceFixture.captures.at(-1).readyState === 'ended'`);
      assert.deepEqual(geometry(), before, 'Cancelled joins restore the same targets');
      evaluate(`voiceFixture.captureError = 'NotAllowedError';`);
      click('Join voice');
      wait(`voiceFixture.client.phase === 'failed' && document.querySelector('.voice-error')`);
      assert.deepEqual(geometry(), before, 'Failed joins keep the channel controls stable and retryable');
      assert.equal(evaluate(`return document.querySelector('[aria-label="Join voice"]').getAttribute('aria-disabled');`), 'false');
      screenshot(`channel-${width}-error`);
      evaluate(`voiceFixture.captureError = undefined;`);
      click('Dismiss voice error');
      browser('set', 'media', 'reduced-motion');
      assert.equal(evaluate(`return getComputedStyle(document.querySelector('.channel-join')).transitionDuration;`), '0s');
      browser('set', 'media', 'no-preference');
      assert.deepEqual(geometry(), before);
      screenshot(`channel-${width}-left`);
      if (width === 1280) {
        browser('focus', '[aria-label="Channel sidebar width"]');
        browser('press', 'Home');
        assert.equal(evaluate(`return document.querySelector('.people-panel').getBoundingClientRect().width;`), 220);
        assert.ok(evaluate(`return [...document.querySelectorAll('.channel-line')].every(line => line.querySelector('.channel-join').getBoundingClientRect().right <= line.getBoundingClientRect().right);`), 'Actions must fit at the 220px sidebar minimum');
        assert.ok(evaluate(`return [...document.querySelectorAll('.channel-voice')].every(row => row.innerText.trim() === 'Join voice');`), 'Minimum-width empty rows also omit redundant status text');
        assert.ok(actionContentsFit(), 'Icons and labels must fit at the 220px sidebar minimum');
        checkChannelTargets();
        screenshot('channel-minimum-sidebar');
        evaluate(`document.activeElement.blur();`);
        const [x, y] = evaluate(`const r = document.querySelector('.channel-sidebar-resize').getBoundingClientRect(); return [Math.round(r.left + r.width / 2), Math.round(r.top + 100)];`);
        browser('mouse', 'move', String(x), String(y));
        browser('mouse', 'down', 'left');
        browser('mouse', 'move', String(x + 67), String(y));
        assert.equal(evaluate(`return document.querySelector('.people-panel').getBoundingClientRect().width;`), 287, 'Pointer dragging must still resize the sidebar');
        assert.equal(evaluate(`return getComputedStyle(document.querySelector('.channel-sidebar-resize')).cursor;`), 'col-resize');
        assert.equal(evaluate(`return getComputedStyle(document.querySelector('.channel-sidebar-resize'), '::after').content;`), 'none', 'Hover/drag must not create a colored resize line');
        screenshot('channel-sidebar-drag-no-highlight');
        browser('mouse', 'up', 'left');
        browser('focus', '[aria-label="Channel sidebar width"]'); browser('press', 'Home');
        browser('press', 'ArrowRight'); browser('press', 'ArrowRight'); browser('press', 'ArrowRight'); browser('press', 'ArrowRight');
      } else {
        assert.ok(before.every(row => row[1][2] >= 44 && row[1][3] >= 44 && row[2][3] >= 44), 'Narrow controls need 44px tap targets');
      }
    }
    const ownerNarrowCount = countPosition();
    browser('set', 'viewport', '1280', '900', '2');
    evaluate(`await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`);
    const ownerDesktopCount = countPosition();
    browser('hover', '.channel-section-heading');
    browser('mouse', 'move', '5', '5');
    assert.deepEqual(countPosition(), ownerDesktopCount, 'Revealing owner section actions cannot shift the count');
    screenshot('channel-1280-owner');
    evaluate(`voiceFixture.showSpaces(false);`);
    wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === 'alpha' && document.querySelector('.channel-manage')`);
    browser('click', '[aria-label="Manage alpha"]');
    assert.equal(evaluate(`return document.querySelector('.channel-menu[open]').textContent.includes('Channel settings');`), false, 'Members cannot access owner settings');
    assert.equal(evaluate(`return document.querySelector('.channel-menu[open]').textContent.includes('Leave channel');`), true, 'Members can leave from channel options');
    browser('press', 'Escape');
    assert.equal(evaluate(`return document.querySelectorAll('.channel-join').length;`), 3, 'Members retain voice actions alongside channel options');
    assert.deepEqual(countPosition(), ownerDesktopCount, 'Owned and shared spaces keep the channel count in the same position');
    checkChannelTargets();
    screenshot('channel-1280-member');
    browser('set', 'viewport', '390', '900', '2');
    browser('find', 'role', 'button', 'click', '--name', 'Browse', '--exact');
    assert.deepEqual(countPosition(), ownerNarrowCount, 'Narrow owned and shared spaces also align the count');
    screenshot('channel-390-member');
    evaluate(`await voiceFixture.cleanup();`);
    console.log('PASS quiet stable channel rows at 1280px/390px and 220px sidebar, empty-status omission, neutral actions and visible keyboard focus, owned/shared count alignment, empty/live/private channels, owner menu and focus return, join/leave/switch/cancel, browsing preserves voice, busy guard, reduced motion and member permissions (mock signaling/WebRTC)');
  } else {
  for (const label of ['Input Options', 'Output Options', 'User Settings']) {
    click(label);
    for (const position of ['top', 'side', 'bottom']) {
      const [x, y] = evaluate(`const r = document.querySelector('.call-settings-panel').getBoundingClientRect(); return ${JSON.stringify(position)} === 'top' ? [Math.round(r.left + r.width / 2), Math.round(r.top + 5)] : ${JSON.stringify(position)} === 'side' ? [Math.round(r.right - 5), Math.round(r.top + r.height / 2)] : [Math.round(r.left + r.width / 2), Math.round(r.bottom - 5)];`);
      browser('mouse', 'move', String(x), String(y)); browser('mouse', 'down', 'left'); browser('mouse', 'up', 'left');
      assert.equal(evaluate(`return document.querySelector('button[aria-label="${label}"]').getAttribute('aria-expanded');`), 'true', 'Empty-space clicks must leave the menu open');
      assert.ok(evaluate(`return document.querySelector('.call-settings-panel').contains(document.activeElement);`), 'Panel background safely takes focus');
    }
    browser('press', 'Escape');
    assert.equal(evaluate(`return document.activeElement.getAttribute('aria-label');`), label);
    assert.equal(evaluate(`return !!document.querySelector('.call-settings-panel');`), false);
    click(label);
    browser('click', '.chat-heading');
    assert.equal(evaluate(`return !!document.querySelector('.call-settings-panel');`), false, 'Outside click still dismisses settings');
  }
  console.log('PASS empty-panel clicks, Escape focus restoration, and outside dismissal');
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
  click('Input Options');
  assert.equal(evaluate(`return !!document.querySelector('.call-settings-panel select');`), true, 'Device menu must use a dropdown');
  browser('focus', '[aria-label="Input volume"]'); browser('press', 'End');
  assert.equal(evaluate(`return document.querySelector('[aria-label="Input volume"]').getAttribute('aria-valuenow');`), '200');
  browser('press', 'Home');
  assert.equal(evaluate(`return document.querySelector('[aria-label="Input volume"]').getAttribute('aria-valuenow');`), '0');
  browser('press', 'End');
  wait(`getComputedStyle(document.querySelector('button[aria-label="Input Options"] svg')).transform === 'matrix(-1, 0, 0, -1, 0, 0)'`);
  assert.equal(evaluate(`return getComputedStyle(document.querySelector('button[aria-label="Input Options"] svg')).transform;`), 'matrix(-1, 0, 0, -1, 0, 0)', 'Open chevron must point up');
  browser('click', 'select[name="input-device"]');
  screenshot('voice-input-dropdown');
  browser('press', 'Escape');
  assert.ok(evaluate(`return !!document.querySelector('.call-settings-panel');`), 'Escape closes the native dropdown before its parent panel');
  browser('select', 'select[name="input-device"]', 'headset');
  assert.equal(evaluate(`return document.querySelector('select[name="input-device"]').value;`), 'headset');
  screenshot('voice-input-options');
  click('Output Options');
  assert.equal(evaluate(`return document.querySelector('button[aria-label="Input Options"]').getAttribute('aria-expanded');`), 'false');
  browser('select', 'select[name="output-device"]', 'headphones');
  assert.equal(evaluate(`return document.querySelector('select[name="output-device"]').value;`), 'headphones');
  browser('focus', '[aria-label="Output volume"]'); browser('press', 'PageDown');
  assert.equal(evaluate(`return document.querySelector('.call-settings-panel .output-volume output').textContent;`), '90%');
  screenshot('voice-output-options');
  browser('press', 'Escape');
  assert.equal(evaluate(`return document.querySelector('button[aria-label="Output Options"]').getAttribute('aria-expanded');`), 'false');
  click('User Settings');
  assert.equal(evaluate(`return !!document.querySelector('.call-settings-panel [role="slider"]');`), false);
  assert.equal(evaluate(`return document.querySelector('.call-settings-panel').textContent.includes('@fixture');`), false);
  assert.equal(evaluate(`return !!document.querySelector('.call-settings-panel a[href="/profile"]');`), false);
  assert.ok(evaluate(`return [...document.querySelectorAll('.call-settings-panel button')].every(button => { const style = getComputedStyle(button); return style.borderTopWidth === '0px' && style.backgroundColor === 'rgba(0, 0, 0, 0)'; });`), 'Menu actions must not look like bordered inputs');
  screenshot('voice-settings');
  browser('press', 'Escape');

  click('Mute microphone');
  assert.equal(evaluate(`return document.querySelector('[aria-label="Unmute microphone"]').getAttribute('aria-pressed');`), 'true', 'Mute must work before joining');
  click('Deafen audio');
  assert.equal(evaluate(`return document.querySelector('[aria-label="Undeafen audio"]').getAttribute('aria-pressed');`), 'true', 'Deafen must work before joining');
  browser('hover', '[aria-label="Undeafen audio"]');
  wait(`getComputedStyle(document.querySelector('[aria-label="Undeafen audio"]')).color === 'rgb(255, 113, 130)'`);
  assert.equal(evaluate(`return getComputedStyle(document.querySelector('[aria-label="Output Options"]')).color;`), 'rgb(237, 82, 101)', 'Dropdown must share the active red state');
  screenshot('voice-prejoin-deafened-hover');
  click('Undeafen audio');
  if (evaluate(`return !!document.querySelector('[aria-label="Unmute microphone"]');`)) click('Unmute microphone');
  click('Join voice');
  wait(`document.querySelector('[aria-label="Leave voice"]')`);
  assert.equal(evaluate(`return voiceFixture.sockets.length;`), 1, 'Chat and joined voice must share one socket');
  assert.deepEqual(bounds(), initialBounds, 'Joining must not move the chat header or composer');
  assert.equal(evaluate(`return voiceFixture.devices.at(-1);`), 'headset');
  evaluate(`voiceFixture.addRemoteAudio();`);
  wait(`voiceFixture.outputGain`);
  const gain = () => evaluate(`return voiceFixture.outputGain.gain.value;`);
  assert.ok(Math.abs(gain() - 0.9) < 0.00001, 'Master volume must reach the audio graph');
  if (evaluate(`return matchMedia('(hover: hover) and (pointer: fine)').matches;`)) {
    browser('click', '.chat-heading');
    assert.equal(evaluate(`return getComputedStyle(document.querySelector('.participant-menu-button')).opacity;`), '0');
    browser('focus', '.participant-menu-button');
    wait(`document.querySelector('.participant-menu-button').matches(':focus') && getComputedStyle(document.querySelector('.participant-menu-button')).opacity === '1'`);
    assert.equal(evaluate(`return getComputedStyle(document.querySelector('.participant-menu-button')).opacity;`), '1', 'Keyboard focus reveals Audio');
    browser('click', '.chat-heading');
  }
  browser('hover', '.participant:has(.participant-menu-button)');
  assert.equal(evaluate(`return getComputedStyle(document.querySelector('.participant-menu-button')).opacity;`), '1');
  click('Audio controls for Peach Donkey');
  browser('focus', '[aria-label="Peach Donkey volume"]'); browser('press', 'PageUp');
  wait(`document.querySelector('[aria-label="Peach Donkey volume"]').getAttribute('aria-valuenow') === '110' && Math.abs(voiceFixture.outputGain.gain.value - 0.99) < 0.00001`);
  assert.ok(Math.abs(gain() - 0.99) < 0.00001, '90% master × 110% participant must produce 99% gain');
  screenshot('participant-audio');
  browser('click', '.chat-heading');
  assert.equal(evaluate(`return !!document.querySelector('.participant-volume');`), false, 'Outside click closes participant controls');
  for (const key of ['Home', 'End', 'Home', 'End']) {
    browser('focus', '.channel-sidebar-resize'); browser('press', key);
    browser('click', '.voice-stack');
    assert.equal(evaluate(`return document.querySelector('.voice-stack').getAttribute('aria-expanded');`), 'false');
    browser('click', '.voice-stack');
    assert.equal(evaluate(`return document.querySelector('.voice-stack').getAttribute('aria-expanded');`), 'true');
  }
  for (const width of [250, 330, 440]) {
    const [x, y, target] = evaluate(`const r = document.querySelector('.channel-sidebar-resize').getBoundingClientRect(); const p = document.querySelector('.people-panel').getBoundingClientRect(); return [r.left + r.width / 2, r.top + 100, p.left + ${width}];`);
    browser('mouse', 'move', String(x), String(y)); browser('mouse', 'down', 'left');
    browser('mouse', 'move', String(target), String(y)); browser('mouse', 'up', 'left');
    browser('click', '.voice-stack');
    assert.equal(evaluate(`return document.querySelector('.voice-stack').getAttribute('aria-expanded');`), 'false');
    browser('click', '.voice-stack');
    assert.equal(evaluate(`return document.querySelector('.voice-stack').getAttribute('aria-expanded');`), 'true');
  }
  click('Input Options');
  assert.ok(evaluate(`return document.querySelector('.call-settings-panel').getBoundingClientRect().width <= 280;`), 'Input menu stays compact in a wide sidebar');
  screenshot('compact-input-wide-sidebar');
  browser('press', 'Escape');
  browser('hover', '.participant:has(.participant-menu-button)');
  click('Audio controls for Peach Donkey');
  browser('press', 'Escape');
  assert.equal(evaluate(`return !!document.querySelector('.participant-volume');`), false);
  click('Audio controls for Peach Donkey');
  click('Audio controls for Peach Donkey');
  click('Output Options'); browser('focus', '[aria-label="Output volume"]'); browser('press', 'Home');
  wait(`voiceFixture.outputGain.gain.value === 0`);
  assert.equal(gain(), 0, 'Zero master volume must silence playback');
  browser('press', 'End');
  wait(`Math.abs(voiceFixture.outputGain.gain.value - 2.2) < 0.00001`);
  assert.ok(Math.abs(gain() - 2.2) < 0.00001, '200% master × 110% participant must produce 220% gain');
  browser('press', 'PageDown'); browser('press', 'Escape');
  click('Mute microphone'); wait(`document.querySelector('[aria-label="Unmute microphone"]')`);
  click('Deafen audio'); wait(`document.querySelector('[aria-label="Undeafen audio"]')`);
  evaluate(`clearInterval(voiceFixture.client.statsTimer);`);
  evaluate(`voiceFixture.client.diagnostics = { join: 'Fixture join', microphoneSessionMs: 10, signalingMs: 20, transportMs: 30, rosterMs: 40, receivedBytes: 10000, receiveBitrate: 32000, sentBytes: 10000, sendBitrate: 32000, packetsLost: 0, maxJitterMs: 1, roundTripMs: 12, route: 'direct' }; voiceFixture.client.emit();`);
  click('User Settings'); browser('find', 'role', 'button', 'click', '--name', 'Connection details', '--exact');
  wait(`document.querySelector('dialog[open] .call-diagnostics')`);
  assert.deepEqual(bounds(), initialBounds, 'Diagnostics must overlay rather than reflow messages');
  centeredDialog();
  evaluate(`navigator.clipboard.writeText = async text => { voiceFixture.copied = text; };`);
  browser('find', 'role', 'button', 'click', '--name', 'Copy connection details', '--exact');
  assert.equal(evaluate(`return JSON.parse(voiceFixture.copied).signalingMs;`), 20);
  assert.ok(evaluate(`return document.querySelector('dialog[open]').textContent.includes('Copied connection details');`));
  screenshot('voice-connection-details');
  browser('mouse', 'move', '5', '5'); browser('mouse', 'down', 'left'); browser('mouse', 'up', 'left');
  wait(`!document.querySelector('dialog[open]')`);
  assert.equal(evaluate(`return document.activeElement.getAttribute('aria-label');`), 'User Settings', 'Dialog close should restore focus to its settings trigger');

  click('User Settings'); browser('find', 'role', 'button', 'click', '--name', 'Audio diagnostics', '--exact');
  wait(`document.querySelector('dialog[open] .audio-debug')`);
  browser('click', '.audio-debug p:first-child');
  assert.ok(evaluate(`return !!document.querySelector('dialog[open]');`), 'Inside click must not dismiss diagnostics');
  browser('mouse', 'move', '5', '5'); browser('mouse', 'down', 'left'); browser('mouse', 'up', 'left');
  wait(`!document.querySelector('dialog[open]')`);

  evaluate(`
    voiceFixture.effectStarts = 0;
    voiceFixture.effectDurations = [];
    const start = AudioBufferSourceNode.prototype.start;
    AudioBufferSourceNode.prototype.start = function (...args) { voiceFixture.effectStarts++; voiceFixture.effectDurations.push(this.buffer.duration); return start.apply(this, args); };
    voiceFixture.client.participants = voiceFixture.client.participants.filter(person => person.id !== 'peer');
    voiceFixture.client.emit();
  `);
  wait(`voiceFixture.effectStarts === 1`);
  evaluate(`voiceFixture.client.participants = [...voiceFixture.client.participants]; voiceFixture.client.emit(); await new Promise(r => setTimeout(r, 200));`);
  assert.equal(evaluate(`return voiceFixture.effectStarts;`), 1, 'A departure plays once; repeated snapshots do not repeat it');
  evaluate(`voiceFixture.client.participants = [...voiceFixture.people]; voiceFixture.client.emit(); await new Promise(r => setTimeout(r, 200));`);
  assert.equal(evaluate(`return voiceFixture.effectStarts;`), 1, 'An arrival must not play the departure sound');
  console.log('PASS participant audio visibility/dismissal, resized roster toggles, compact menu width, diagnostics copy/dismissal, and departure sound (mock signaling/WebRTC)');

  click('User Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
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
  click('Input Options');
  assert.ok(evaluate(`const r = document.querySelector('.call-settings-panel').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth;`));
  screenshot('voice-controls-narrow');
  browser('press', 'Escape');
  click('Output Options');
  assert.ok(evaluate(`const r = document.querySelector('.call-settings-panel').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth;`));
  screenshot('voice-output-narrow');
  browser('press', 'Escape');
  click('User Settings'); screenshot('voice-settings-narrow'); browser('press', 'Escape');
  click('User Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
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
  const beforeDisconnect = evaluate(`return voiceFixture.effectStarts;`);
  click('Leave voice'); wait(`document.querySelector('[aria-label="Join voice"]')`);
  wait(`voiceFixture.effectStarts === ${beforeDisconnect + 1}`);
  assert.ok(evaluate(`return Math.abs(voiceFixture.effectDurations.at(-1) - 0.890159) < 0.001;`), 'Self-disconnect plays the supplied 0.89-second WAV');
  assert.notEqual(evaluate(`return voiceFixture.effectDurations[0];`), evaluate(`return voiceFixture.effectDurations.at(-1);`), 'Remote departure and self-disconnect must be distinct sounds');
  console.log('PASS supplied self-disconnect sound is distinct from remote departure');
  click('User Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] .mic-test-button')`);
  click('Close audio settings'); wait(`!document.querySelector('dialog[open]')`);
  assert.equal(evaluate(`return voiceFixture.captures.at(-1).readyState;`), 'ended', 'Closing a pre-join test must release its microphone');
  evaluate(`voiceFixture.holdCapture = true;`);
  click('User Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] [role="status"]')`);
  click('Close audio settings');
  evaluate(`voiceFixture.holdCapture = false; voiceFixture.captureError = 'NotFoundError';`);
  click('User Settings'); browser('find', 'role', 'button', 'click', '--name', 'Mic test', '--exact');
  wait(`document.querySelector('dialog[open] [role="alert"]')`);
  assert.ok(evaluate(`return document.querySelector('dialog[open]').textContent.includes('Connect a microphone');`));
  assert.equal(evaluate(`return !!document.querySelector('dialog[open] > [role="status"]');`), false, 'Mic preparation status must clear after capture fails');
  evaluate(`voiceFixture.captureError = undefined;`);
  browser('find', 'role', 'button', 'click', '--name', 'Try again', '--exact');
  wait(`document.querySelector('dialog[open] .mic-test-button')`);
  evaluate(`voiceFixture.releaseCapture();`);
  wait(`voiceFixture.captures.at(-1).readyState === 'ended'`);
  assert.equal(evaluate(`return !!document.querySelector('dialog[open] .mic-test-button');`), true, 'Late cancelled capture must not disturb the successful retry');
  click('Close audio settings');
  assert.equal(evaluate(`return !!document.querySelector('.call-controls') || /huddle/i.test(document.querySelector('.call-page').textContent);`), false);
  browser('set', 'viewport', '1280', '900', '2');
  evaluate(`voiceFixture.showSpaces();`);
  wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === 'alpha' && document.querySelector('.voice-button[aria-disabled="false"]')`);
  click('Mute microphone');
  click('Join voice');
  wait(`document.querySelector('[aria-label="Leave voice"]')`);
  assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).body.muted;`), true, 'Join must transmit pre-join mute intent');
  evaluate(`voiceFixture.joinedClient = voiceFixture.client; voiceFixture.callTrack = voiceFixture.captures.at(-1); voiceFixture.leaveCount = voiceFixture.commands.filter(c => c.method === 'media.leave').length; voiceFixture.settingsNode = document.querySelector('[aria-label="User Settings"]');`);
  browser('find', 'role', 'button', 'click', '--name', 'beta', '--exact');
  wait(`document.querySelector('.channel-select[aria-current="page"]')?.textContent === 'beta' && document.querySelector('.voice-button[aria-disabled="false"]')`);
  assert.ok(evaluate(`return voiceFixture.client === voiceFixture.joinedClient && voiceFixture.callTrack.readyState === 'live';`), 'Browsing beta must keep alpha voice alive');
  assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`), evaluate(`return voiceFixture.leaveCount;`), 'Navigation must not send Leave');
  assert.ok(evaluate(`return document.querySelector('.connected-channel').textContent.includes('Test space / alpha');`), 'Connected channel must remain visible');
  assert.ok(evaluate(`return document.querySelector('[aria-label="User Settings"]') === voiceFixture.settingsNode;`), 'Settings must not remount on navigation');
  assert.ok(evaluate(`return !!document.querySelector('[aria-label="Switch voice to #beta"]') && !!document.querySelector('[aria-label="Unmute microphone"]');`));
  screenshot('voice-browsing-another-channel');
  browser('click', '.voice-dock-channel');
  assert.equal(evaluate(`return document.querySelector('.channel-select[aria-current="page"]')?.textContent;`), 'beta', 'Connection status must not act as channel navigation');
  assert.ok(evaluate(`return !!document.querySelector('[aria-label="Leave voice"]');`));
  click('Switch voice to #beta');
  wait(`document.querySelector('[aria-label="Leave voice"]')`);
  assert.equal(evaluate(`return voiceFixture.callTrack.readyState;`), 'ended', 'Explicit Join must release the previous call');
  assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`), evaluate(`return voiceFixture.leaveCount + 1;`));
  assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').at(-1).channelId;`), 'alpha0000000');
  assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).channelId;`), 'beta00000000');
  assert.ok(evaluate(`return document.querySelector('.connected-channel').textContent.includes('Test space / beta') && voiceFixture.client.muted;`), 'Voice switch must preserve mute');
  click('Leave voice');
  wait(`!document.querySelector('.connected-channel')`);
  evaluate(`voiceFixture.captureError = 'NotReadableError';`);
  click('Join voice');
  wait(`document.querySelector('.room-error')`);
  assert.ok(evaluate(`return document.querySelector('.room-error').textContent.includes('another app');`), 'Busy microphone errors must be actionable');
  assert.ok(evaluate(`return document.querySelector('.chat-heading').getBoundingClientRect().top === document.querySelector('.channel-navigation > header').getBoundingClientRect().top;`), 'Voice errors must not shift the channel header');
  screenshot('voice-device-busy-error');
  evaluate(`voiceFixture.captureError = undefined; voiceFixture.holdCapture = true; voiceFixture.releaseCapture = undefined;`);
  click('Join voice');
  wait(`document.querySelector('[aria-label="Cancel joining voice"]') && voiceFixture.releaseCapture`);
  click('Cancel joining voice');
  wait(`document.querySelector('[aria-label="Join voice"]')`);
  evaluate(`voiceFixture.holdCapture = false; voiceFixture.releaseCapture();`);
  wait(`voiceFixture.captures.at(-1).readyState === 'ended'`);
  assert.equal(evaluate(`return voiceFixture.client.phase;`), 'idle', 'Late microphone capture must not resurrect a cancelled join');
  evaluate(`await voiceFixture.cleanup();`);
  console.log('PASS voice navigation persistence, explicit channel switching, pre-join mute/deafen and red hover/dropdown states, busy-device errors, join cancellation, profile, audio settings, recording cleanup, and narrow layout (mock signaling/WebRTC)');
  }
} catch (error) {
  screenshot('failure');
  console.error(browser('errors'));
  console.error(evaluate(`return { alerts: [...document.querySelectorAll('[role="alert"]')].map(node => node.textContent), phase: window.voiceFixture?.client?.phase, channels: [...document.querySelectorAll('.channel-select')].map(node => [node.textContent, node.getAttribute('aria-current')]), voice: [...document.querySelectorAll('.voice-button')].map(node => [node.textContent, node.getAttribute('aria-disabled')]), pages: document.querySelectorAll('.call-page').length };`));
  throw error;
} finally {
  browser('close');
}
