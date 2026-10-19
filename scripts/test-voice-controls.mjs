// Real Call/Chat components and voice client; synthetic audio and mocked HTTP,
// WebSocket and WebRTC. This is UI regression coverage, not live SFU validation.
// Run against Vite: node scripts/test-voice-controls.mjs [http://localhost:5174]
// Desktop hover states need VOICE_TEST_CHROME="$PWD/scripts/fine-pointer-chromium.sh".
// Focus on DM navigation, hover menus and ringless focus with VOICE_TEST_SIDEBAR=1.
// Focus on stable channel rows with VOICE_TEST_CHANNEL_ROWS=1.
// Focus only on avatar borders with VOICE_TEST_AVATARS=1.
// Focus on the connected account bar and call card links with VOICE_TEST_DOCK=1.
// Focus on click-time/shared timers with VOICE_TEST_TIMERS=1.
// Check live channels (call card and room cards in the list, row joins, phone strip) with VOICE_TEST_REDESIGN=1.
// Check space-rail voice badges across navigation and connection states with VOICE_TEST_RAIL=1.
// Serve an interactive, explicitly simulated preview with VOICE_TEST_PREVIEW=1.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createServer } from "node:http";
import { resolve } from "node:path";

const origin = new URL(process.argv[2] ?? "http://localhost:5174");
assert.ok(["localhost", "127.0.0.1"].includes(origin.hostname), "Use local Vite, never production");
const session = `voice-ui-${process.pid}`;
const artifacts = process.env.VOICE_TEST_ARTIFACTS && resolve(process.env.VOICE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
function browser(...args) {
  const result = JSON.parse(
    execFileSync(
      "agent-browser",
      [
        "--session",
        session,
        ...(process.env.VOICE_TEST_CHROME ? ["--executable-path", process.env.VOICE_TEST_CHROME] : []),
        "--args",
        "--autoplay-policy=no-user-gesture-required",
        ...args,
        "--json",
      ],
      { encoding: "utf8", timeout: 60000 },
    ),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (code) => browser("eval", `(async () => { ${code} })()`).result;
const wait = (code) => browser("wait", "--fn", `Boolean(${code})`);
// Browse slides the conversation with a view transition; clicks and hit tests
// during it land on its snapshot, so settle any slide before and after.
const settleBrowse = () => wait(`!document.querySelector(".spaces-room")?.hasAttribute("data-browse-slide")`);
const click = (label) => {
  settleBrowse();
  browser("click", `[aria-label="${label}"]`);
  if (["Back to Browse", "Close navigation"].includes(label)) settleBrowse();
};
const screenshot = (name) => {
  if (!artifacts) return;
  evaluate(
    `await document.fonts.ready; await new Promise(r => setTimeout(r, 200)); await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`,
  );
  browser("screenshot", "--full", `${artifacts}/${name}.png`);
};
// Phones: let any Browse slide finish, open Browse if it closed, then let that
// slide settle too (Browse ignores clicks while it moves).
const openBrowse = () => {
  browser("wait", "600");
  if (evaluate(`return !document.querySelector('.navigation-open');`)) click("Back to Browse");
  browser("wait", "600");
};
// Whether this channel's row in the list is your call's card.
const connectedRow = (name) =>
  `[...document.querySelectorAll('#space-channel-list > li')].some(li => li.querySelector('.live-call .channel-select > span')?.textContent === ${JSON.stringify(name)})`;
// Opens a channel from its row's button, whose name also carries a live row's "N in voice".
const chooseChannel = (name) => {
  evaluate(`document.querySelector('[data-test-row]')?.removeAttribute('data-test-row');
    [...document.querySelectorAll('.channel-select')].find(button => button.querySelector('span').textContent === ${JSON.stringify(name)}).setAttribute('data-test-row', '');`);
  browser("click", "[data-test-row]");
};
// A right-click at an element's center: what opens everyone's volume from a call card's faces.
const rightClick = (selector) => {
  const [x, y] = evaluate(
    `const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return [Math.round(r.x + r.width / 2), Math.round(r.y + r.height / 2)];`,
  );
  browser("mouse", "move", String(x), String(y));
  browser("mouse", "down", "right");
  browser("mouse", "up", "right");
};
// A touch long press, then the click it ends with (synthetic: headless Chromium has no touchscreen).
const longPress = (selector) =>
  evaluate(`const target = document.querySelector(${JSON.stringify(selector)}), r = target.getBoundingClientRect();
    const at = { bubbles: true, pointerType: 'touch', pointerId: 7, isPrimary: true, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2 };
    target.dispatchEvent(new PointerEvent('pointerdown', at));
    await new Promise(resolve => setTimeout(resolve, 650));
    target.dispatchEvent(new PointerEvent('pointerup', at));
    target.dispatchEvent(new MouseEvent('click', at));`);
// Faces by avatar artwork (or initials), to compare the call card's with the phone strip's.
const faceIds = (selector) =>
  `[...document.querySelectorAll(${JSON.stringify(selector)})].map(face => face.querySelector('[data-avatar-id]')?.dataset.avatarId ?? face.textContent)`;
const centeredDialog = () => {
  // Measure after the dialog's entrance animation (a short translate) settles.
  wait(`!document.querySelector('dialog[open]').getAnimations().some(a => a.playState === 'running')`);
  assert.ok(
    evaluate(
      // Center within the page width: the root reserves a stable scrollbar gutter (#465).
      `const r = document.querySelector('dialog[open]').getBoundingClientRect(), width = document.documentElement.getBoundingClientRect().width; return Math.abs(r.left + r.width / 2 - width / 2) < 1 && Math.abs(r.top + r.height / 2 - innerHeight / 2) < 1;`,
    ),
    "Dialog must stay centered within the viewport",
  );
};

async function fixture() {
  const { default: React } = await import("/node_modules/.vite/deps/react.js");
  const {
    default: { createRoot },
  } = await import("/node_modules/.vite/deps/react-dom_client.js");
  // Use Call's exact module URL, including Vite's HMR cache key after edits.
  const source = await (await fetch("/src/pages/Call.tsx")).text();
  const { PublicCallClient } = await import(source.match(/from "(\/src\/media\/client\.ts[^"]*)"/)[1]);
  const { default: Call } = await import("/src/pages/Call.tsx");
  const { default: Spaces } = await import("/src/spaces/Spaces.tsx");
  // Remove the homepage's simulated channels from document-level test queries.
  // oxlint-disable-next-line unicorn/no-useless-spread -- Snapshot the live HTMLCollection before removing children.
  for (const element of [...document.body.children]) element.remove();
  const mount = document.createElement("div");
  document.body.append(mount);
  const f = (window.voiceFixture = {
    people: [],
    sockets: [],
    captures: [],
    devices: [],
    recorders: [],
    commands: [],
    revision: 0,
  });
  const account = {
    id: "fixture-user",
    username: "fixture",
    displayName: "UI fixture",
    avatarId: 0,
    debugEnabled: true,
  };
  const selfDirect = { id: "selfdm000000", peer: account, lastSeq: "0", readSeq: "0", status: "accepted" };
  const space = { id: "workspace123", name: "Test space", ownerId: account.id };
  const otherSpace = { id: "otherSpace12", name: "Other test space", ownerId: account.id };
  const otherChannel = { id: "lobby0000000", name: "lobby", spaceId: otherSpace.id, private: false };
  const channels = ["alpha", "beta"].map((name) => ({
    id: name.padEnd(12, "0"),
    name,
    spaceId: space.id,
    private: false,
  }));
  channels.push({ id: "private00000", name: "planning-for-the-next-release", spaceId: space.id, private: true });
  const author = { id: account.id, name: account.displayName, isGuest: false };
  const peer = { id: "peer", name: "Peach Donkey", isGuest: true };
  const snapshot = (channelId) => {
    const visible = !f.timerEnabled || !channelId || channelId === channels[0].id;
    return {
      participants: visible ? f.people : [],
      revision: f.revision,
      ...(f.timerEnabled ? { sessionStartedAt: visible ? (f.sessionStartedAt ?? null) : null } : {}),
    };
  };
  const originalFetch = window.fetch.bind(window);
  const respond = async (input, options = {}) => {
    const path = new URL(typeof input === "string" ? input : input.url, location.href).pathname;
    if (path === "/api/account/me") return Response.json(account);
    // Profile settings load notification, DM privacy and block settings.
    if (path === "/api/notifications/settings")
      return Response.json({ level: "all", mobile: "whenInactive", overrides: [] });
    if (path === "/api/account/privacy") return Response.json({ directMessages: "anyone" });
    if (path === "/api/blocks") return Response.json({ blocks: [] });
    if (path === "/api/dms") return Response.json({ conversations: [selfDirect] });
    if (path === "/api/people") return Response.json({ people: [account] });
    if (path === "/api/spaces/voice-activity")
      return f.activityError
        ? Response.json({ error: "Fixture outage" }, { status: 503 })
        : Response.json({ spaceIds: f.activeSpaces ?? [] });
    if (path === "/api/spaces")
      return Response.json({
        spaces: f.noSpaces ? [] : [space, otherSpace],
        invitations: f.invitations ?? [],
        limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
      });
    if (path === "/api/spaces/workspace123")
      return Response.json({ space, channels, members: [{ ...account, owner: true }] });
    if (path === "/api/spaces/otherSpace12")
      return Response.json({
        space: otherSpace,
        channels: f.otherDeleted ? [] : [{ ...otherChannel, ...(f.otherUnjoined ? { joined: false } : {}) }],
        members: [{ ...account, owner: true }],
      });
    if (path === "/api/spaces/workspace123/channels/private00000/members") return Response.json({ members: [account] });
    if (path.startsWith("/api/chat/channels/") && path.endsWith("/messages"))
      return Response.json({
        // DM history belongs to no space (unified DMs).
        space: path.includes(`/${selfDirect.id}/`)
          ? { id: "", name: "Direct messages" }
          : path.includes(otherChannel.id)
            ? otherSpace
            : space,
        channel: path.includes(`/${selfDirect.id}/`)
          ? { id: selfDirect.id, name: account.displayName, direct: true }
          : [...channels, otherChannel].find((channel) => path.includes(`/${channel.id}/`)),
        messages: [],
        cursor: "0",
        hasMore: false,
      });
    if (path.endsWith("/media/status")) return Response.json({ enabled: true });
    if (path === "/api/account/profile") {
      const updated = JSON.parse(options.body);
      if (updated.username === "taken") return Response.json({ error: "Taken" }, { status: 409 });
      Object.assign(account, updated);
      author.name = updated.displayName;
      return Response.json(account);
    }
    if (path === "/api/chat/session") return Response.json({ token: "fixture-only", author });
    if (path === "/api/chat/general")
      return Response.json({
        space: { id: "fixture", name: "UI fixture" },
        channel: { id: "general", name: "General" },
        cursor: "1",
        hasMore: false,
        messages: [
          {
            id: "example",
            channelId: "general",
            seq: "1",
            clientMessageId: "fixture-message",
            author: peer,
            content: { version: 1, type: "text", text: "UI test fixture · synthetic audio, no live participants." },
            createdAt: "2026-09-21T16:00:00Z",
          },
        ],
      });
    if (path.endsWith("/typing")) return new Response(null, { status: 204 });
    if (path === "/api/media/status") return Response.json({ enabled: true });
    // No release highlights in this fixture; What's new stays closed.
    if (path === "/api/whats-new") return Response.json({ error: "Fixture" }, { status: 404 });
    if (path === "/api/media/presence" || path === "/api/media/snapshot")
      return Response.json(snapshot(options.channelId));
    if (path === "/api/media/join") {
      if (f.holdJoin)
        await new Promise((resolve) => {
          f.releaseJoin = resolve;
        });
      if (f.timerEnabled) f.sessionStartedAt ??= JSON.parse(options.body).joinStartedAt;
      const self = {
        id: "self",
        name: account.displayName,
        avatarId: account.avatarId,
        muted: false,
        deafened: false,
        tracks: [],
      };
      f.people = f.timerEnabled ? [...f.people.filter((person) => person.id !== "self"), self] : [self];
      f.revision++;
      if (f.timerEnabled) f.publishPresence(f.people);
      return Response.json({ token: "fixture-voice", id: "self", iceServers: [] });
    }
    if (path === "/api/media/publish") return Response.json({ sessionDescription: { type: "answer", sdp: "v=0" } });
    if (path === "/api/media/warm")
      return Response.json({ error: "Warm voice is not supported by this fixture" }, { status: 422 });
    if (path === "/api/media/leave" && f.timerEnabled) {
      const remaining = f.people.filter((person) => person.id !== "self");
      if (!remaining.length) f.sessionStartedAt = undefined;
      f.publishPresence(remaining);
    }
    if (["/api/media/state", "/api/media/leave", "/api/media/close", "/api/media/prepare"].includes(path))
      return new Response(null, { status: 204 });
    if (path.startsWith("/api/")) throw Error(`Unexpected fixture request: ${path}`);
    return originalFetch(input, options);
  };
  window.fetch = (input, options = {}) => {
    const path = new URL(typeof input === "string" ? input : input.url, location.href).pathname;
    if ((path.startsWith("/api/media/") && path !== "/api/media/status") || path.endsWith("/typing")) {
      throw Error(`Ephemeral operation bypassed gateway: ${path}`);
    }
    return respond(input, options);
  };
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    subscriptions = new Map();
    constructor(url, protocols) {
      super();
      if (!String(url).includes("/api/chat/events")) return new NativeSocket(url, protocols);
      f.sockets.push(this);
      setTimeout(() => this.frame({ type: "hello", idleTimeoutSeconds: 600, serverTime: Date.now() }), 0);
    }
    async send(data) {
      const request = JSON.parse(data);
      if (request.type === "heartbeat") this.frame({ type: "heartbeat" });
      if (request.type === "unsubscribe") this.subscriptions.delete(request.id);
      if (request.type === "subscribe") {
        this.subscriptions.set(request.id, request);
        const room = snapshot(request.channelId);
        const event =
          request.kind === "chat" || request.kind === "notifications"
            ? { type: "ready", cursor: request.after ?? "0" }
            : request.kind === "presence"
              ? { type: "snapshot", members: request.userIds.map((userId) => ({ userId, status: "online" })) }
              : {
                  type: "snapshot",
                  ...room,
                  participants: request.token
                    ? room.participants
                    : room.participants.map(({ tracks, ...person }) => person),
                };
        this.frame({ type: "event", id: request.id, event });
        this.frame({ type: "subscribed", id: request.id });
      }
      if (request.type === "command") {
        f.commands.push(request);
        // Reuse fixture responses, without issuing network requests. The real
        // client must reach them through a command on this shared socket.
        const path =
          request.method === "typing"
            ? `/api/chat/channels/${request.channelId}/typing`
            : `/api/media/${request.method.slice("media.".length)}`;
        const response = await respond(path, { body: JSON.stringify(request.body), channelId: request.channelId });
        this.frame({
          type: "result",
          id: request.id,
          status: response.status,
          body: response.status === 204 ? null : await response.json(),
        });
      }
    }
    frame(event) {
      this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(event) }));
    }
    close() {
      f.sockets = f.sockets.filter((socket) => socket !== this);
    }
  };
  f.typing = () =>
    f.sockets.forEach((socket) => {
      for (const [id, subscription] of socket.subscriptions)
        if (subscription.kind === "chat") {
          socket.frame({
            type: "event",
            id,
            event: {
              type: "typing.updated",
              channelId: "general",
              author: peer,
              typing: true,
              revision: String(Date.now() * 1000),
            },
          });
        }
    });
  f.publishPresence = (people) => {
    f.people = people;
    f.revision++;
    f.sockets.forEach((socket) => {
      for (const [id, subscription] of socket.subscriptions)
        if (subscription.kind === "media" && !subscription.token) {
          const room = snapshot(subscription.channelId);
          socket.frame({
            type: "event",
            id,
            event: {
              type: "snapshot",
              ...room,
              participants: room.participants.map(({ tracks, ...person }) => person),
            },
          });
        }
    });
  };
  const context = new AudioContext();
  await context.resume();
  const signal = context.createOscillator();
  signal.start();
  // Device IDs are synthetic; exercise selection without requiring host hardware.
  HTMLMediaElement.prototype.setSinkId = async function () {};
  AudioContext.prototype.setSinkId = async function () {};
  f.sampleGains = [];
  const createMediaElementSource = AudioContext.prototype.createMediaElementSource;
  AudioContext.prototype.createMediaElementSource = function (element) {
    const source = createMediaElementSource.call(this, element);
    const connect = source.connect.bind(source);
    source.connect = (node) => {
      f.sampleGains.push(node);
      return connect(node);
    };
    return source;
  };
  f.addRemoteAudio = () => {
    const createGain = AudioContext.prototype.createGain;
    // Capture the remote playback gain (it feeds a stream destination), not the
    // gains that join/leave sound effects create at the same time.
    AudioContext.prototype.createGain = function () {
      const node = createGain.call(this);
      const connect = node.connect.bind(node);
      node.connect = (target, ...rest) => {
        if (target instanceof MediaStreamAudioDestinationNode) f.outputGain = node;
        return connect(target, ...rest);
      };
      return node;
    };
    f.people.push({ id: peer.id, name: peer.name, muted: false, deafened: false, tracks: [] });
    f.client.participants = [...f.people];
    f.remoteStream = context.createMediaStreamDestination().stream;
    f.client.remoteMedia.set("remote", {
      trackId: "remote",
      participantId: peer.id,
      kind: "microphone",
      stream: f.remoteStream,
    });
    f.client.emit();
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        AudioContext.prototype.createGain = createGain;
      }),
    );
  };
  navigator.mediaDevices.enumerateDevices = async () => [
    { deviceId: "default", kind: "audioinput", label: "Desk microphone" },
    { deviceId: "headset", kind: "audioinput", label: "Headset microphone" },
    { deviceId: "default", kind: "audiooutput", label: "Speakers" },
    { deviceId: "speakers", kind: "audiooutput", label: "Speakers" },
    { deviceId: "headphones", kind: "audiooutput", label: "Headphones" },
    { deviceId: "bluetooth", kind: "audiooutput", label: "Bluetooth headphones" },
    { deviceId: "monitor", kind: "audiooutput", label: "HDMI monitor" },
  ];
  PublicCallClient.prototype.prepareMicrophone = () => {};
  PublicCallClient.prototype.openMicrophone = async function (deviceId) {
    f.devices.push(deviceId);
    f.client = this;
    if (f.captureError) throw new DOMException("Fixture capture error", f.captureError);
    if (f.holdCapture)
      await new Promise((resolve) => {
        f.releaseCapture = resolve;
      });
    const destination = context.createMediaStreamDestination();
    signal.connect(destination);
    const track = destination.stream.getAudioTracks()[0];
    f.captures.push(track);
    return track;
  };
  window.RTCPeerConnection = class extends EventTarget {
    connectionState = "connected";
    iceGatheringState = "complete";
    senders = [];
    configuration = {};
    getConfiguration() {
      return this.configuration;
    }
    setConfiguration(value) {
      this.configuration = value;
    }
    addTransceiver(track) {
      const sender = {
        track: typeof track === "string" ? null : track,
        async replaceTrack(next) {
          this.track = next;
        },
      };
      this.senders.push(sender);
      return { mid: "0", sender };
    }
    async createOffer() {
      return { type: "offer", sdp: "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\n" };
    }
    async setLocalDescription(value) {
      this.localDescription = { toJSON: () => value };
    }
    async setRemoteDescription() {}
    getSenders() {
      return this.senders;
    }
    getReceivers() {
      return [];
    }
    async getStats() {
      return new Map();
    }
    close() {
      this.connectionState = "closed";
    }
  };
  const Recorder = window.MediaRecorder;
  window.MediaRecorder = class extends Recorder {
    constructor(...args) {
      super(...args);
      f.recorders.push(this);
    }
  };
  localStorage.removeItem("caper.chat.session");
  // Isolate the mounted fixture from the hidden app's router. Spaces still uses
  // real browser history, but must not mount a second copy in the hidden app.
  history.pushState = History.prototype.pushState.bind(history);
  history.replaceState = History.prototype.replaceState.bind(history);
  const root = createRoot(mount);
  root.render(React.createElement(Call));
  f.showSpaces = (owner = true, dmId) => {
    space.ownerId = owner ? account.id : "other-owner";
    history.replaceState({}, "", `/spaces?space=${space.id}&${dmId ? `dm=${dmId}` : `channel=${channels[0].id}`}`);
    root.render(React.createElement(Spaces, { key: `${owner ? "owner" : "member"}:${dmId ?? "channel"}` }));
  };
  f.cleanup = async () => {
    root.unmount();
    f.captures.forEach((track) => track.stop());
    f.remoteStream?.getTracks().forEach((track) => track.stop());
    await context.close();
  };
}

if (process.env.VOICE_TEST_PREVIEW === "1") {
  // Development-only proxy to the loopback Vite server. The production app
  // has no fixture route, fake participants, or preview controls.
  const server = createServer(async (request, response) => {
    try {
      const target = new URL(request.url, origin);
      if (target.origin !== origin.origin || !["GET", "HEAD"].includes(request.method)) {
        response.writeHead(405).end();
        return;
      }
      const upstream = await fetch(target);
      const type = upstream.headers.get("content-type") ?? "application/octet-stream";
      response.writeHead(upstream.status, { "content-type": type, "cache-control": "no-store" });
      if (!type.includes("text/html")) {
        response.end(Buffer.from(await upstream.arrayBuffer()));
        return;
      }
      const html = await upstream.text();
      const bootstrap = `<script type="module">
        const gate = document.createElement('button');
        gate.textContent = 'Open voice UI preview · simulated call, no real microphone';
        gate.style.cssText = 'position:fixed;inset:0;margin:auto;width:min(90vw,480px);height:96px;z-index:1000;border:1px solid #34383B;border-radius:8px;background:#151719;color:#F3F4F5;font:600 16px system-ui;cursor:pointer';
        document.body.append(gate);
        gate.addEventListener('click', async () => {
          gate.disabled = true;
          await (${fixture.toString()})();
          voiceFixture.timerEnabled = true;
          voiceFixture.sessionStartedAt = Date.now() - 37000;
          voiceFixture.publishPresence([
            {id:'peer',name:'Maya',avatarId:31,muted:false,deafened:false,tracks:[]},
            {id:'other',name:'Alex',avatarId:799,muted:true,deafened:false,tracks:[]}
          ]);
          voiceFixture.showSpaces();
          const label = document.createElement('span');
          label.textContent = 'UI preview · simulated voice';
          label.style.cssText = 'position:fixed;top:24px;right:24px;max-width:180px;z-index:20;color:#B9BCBE;font:12px system-ui;text-align:right';
          document.body.append(label);
        }, {once:true});
      </script>`;
      response.end(html.replace("</body>", `${bootstrap}</body>`));
    } catch (error) {
      response.writeHead(502).end(String(error));
    }
  });
  server.listen(Number(process.env.PORT ?? 5180), "0.0.0.0");
  console.log("Voice UI preview: synthetic audio, mocked API/WebRTC, no live service or microphone.");
  for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => server.close(() => process.exit(0)));
  await new Promise(() => {});
}

try {
  browser("open", origin.href);
  browser("set", "viewport", "1280", "900", "2");
  // Wait for Start's document-level hydration before adding a second root.
  // Otherwise hydration can replace the body and discard the fixture mount.
  browser("wait", "1500");
  evaluate(`await (${fixture.toString()})();`);
  wait(
    `document.querySelector('#chat-message:not(:disabled)') && document.querySelector('.channel-join[aria-disabled="false"]')`,
  );
  if (process.env.VOICE_TEST_RAIL === "1") {
    evaluate(`voiceFixture.activeSpaces = ['otherSpace12']; voiceFixture.showSpaces();`);
    const other = '.space-rail button[aria-label^="Other test space"]';
    const own = '.space-rail button[aria-label^="Test space"]';
    wait(`document.querySelector('${other} .space-rail-voice')`);
    assert.equal(evaluate(`return !!document.querySelector('${own} .space-rail-voice');`), false);
    assert.equal(
      evaluate(`return document.querySelector('${other}').getAttribute('aria-label');`),
      "Other test space, voice active",
    );
    screenshot("voice-rail-others-1280");
    evaluate(`voiceFixture.holdCapture = true;`);
    click("Join voice in #alpha");
    wait(`voiceFixture.releaseCapture`);
    assert.equal(
      evaluate(`return document.querySelectorAll('.space-rail-voice[data-connected]').length;`),
      0,
      "Joining is not connected",
    );
    evaluate(`voiceFixture.releaseCapture();`);
    wait(`document.querySelector('${own} .space-rail-voice[data-connected]')`);
    assert.equal(
      evaluate(`return document.querySelector('${own}').getAttribute('aria-label');`),
      "Test space, connected to voice",
    );
    screenshot("voice-rail-connected-1280");
    browser("click", other);
    wait(`document.querySelector('${other}[aria-current="page"]')`);
    assert.ok(
      evaluate(`return !!document.querySelector('${own} .space-rail-voice[data-connected]');`),
      "Browsing another space keeps the connected badge on the call's space",
    );
    assert.equal(evaluate(`return !!document.querySelector('${other} .space-rail-voice[data-connected]');`), false);
    browser("set", "viewport", "390", "844", "2");
    openBrowse();
    screenshot("voice-rail-connected-390");
    evaluate(`voiceFixture.activityError = true; window.dispatchEvent(new Event('focus'));`);
    wait(`!document.querySelector('${other} .space-rail-voice')`);
    assert.ok(
      evaluate(`return !!document.querySelector('${own} .space-rail-voice[data-connected]');`),
      "Poll failures do not hide the actual local connection",
    );
    click("Leave voice");
    wait(`!document.querySelector('.space-rail-voice')`);
    screenshot("voice-rail-empty-390");
    console.log(
      "PASS: voice badges distinguish idle/joining/connected, survive cross-space navigation, render at 1280/390px, clear on outage/leave.",
    );
  } else if (process.env.VOICE_TEST_SIDEBAR === "1") {
    const channelGeometry = () =>
      evaluate(`return [...document.querySelectorAll('.channel-line')].map(line => {
        const r = line.getBoundingClientRect(); return [r.x, r.y, r.width, r.height];
      });`);
    const desktopHover = () => evaluate(`return matchMedia('(hover: hover) and (pointer: fine)').matches;`);
    const menuOpacity = () =>
      evaluate(`return getComputedStyle(document.querySelector('[aria-label="Manage alpha"]')).opacity;`);
    // Phone Browse slides (covered by test-desktop-navigation) swallow input while they run.
    browser("set", "media", "reduced-motion");
    for (const width of [1280, 390]) {
      browser("set", "viewport", String(width), "900", "2");
      evaluate(`voiceFixture.publishPresence([]); voiceFixture.showSpaces(${width === 1280});`);
      wait(`document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha'`);
      if (width === 390) click("Back to Browse");
      evaluate(
        `await document.fonts.ready; await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))); document.activeElement.blur();`,
      );
      browser("mouse", "move", "5", "5");
      const before = channelGeometry();
      assert.equal(before.length, 3);
      assert.equal(
        menuOpacity(),
        width === 1280 && desktopHover() ? "0" : "1",
        "Only mouse desktop hides resting channel menus",
      );
      assert.equal(
        evaluate(`const button = document.querySelector('.channel-line .channel-select');
        return button.querySelector('span').getBoundingClientRect().left - button.querySelector('svg').getBoundingClientRect().right;`),
        6,
        "The channel label sits 6px after its hash/lock",
      );
      screenshot(`sidebar-${width}-default`);
      if (width === 1280 && desktopHover()) {
        browser("hover", "#space-channel-list > li:first-child .channel-line");
        wait(`getComputedStyle(document.querySelector('[aria-label="Manage alpha"]')).opacity === '1'`);
        assert.equal(menuOpacity(), "1", "Hover reveals that channel's menu");
        assert.equal(
          evaluate(`return getComputedStyle(document.querySelector('[aria-label="Manage beta"]')).opacity;`),
          "0",
          "Other rows stay quiet",
        );
        assert.deepEqual(channelGeometry(), before, "Revealing the menu reserves its space");
        screenshot("sidebar-1280-hover");
        click("Manage alpha");
        browser("mouse", "move", "5", "5");
        assert.equal(menuOpacity(), "1", "An open menu stays visible off hover");
        browser("press", "Escape");
      }
      browser("focus", '.channel-select[aria-current="page"]');
      browser("press", "Tab");
      // Fine pointers reach the row's join shortcut before its menu; touch hides it.
      assert.equal(
        evaluate(`return document.activeElement.getAttribute('aria-label');`),
        desktopHover() ? "Join voice in #alpha" : "Manage alpha",
      );
      wait(`getComputedStyle(document.activeElement).backgroundColor === 'rgb(52, 56, 59)'`);
      assert.equal(
        evaluate(`return getComputedStyle(document.activeElement).outlineStyle;`),
        "none",
        "Keyboard focus uses a background, not a ring",
      );
      assert.equal(menuOpacity(), "1", "Keyboard focus reveals channel options");
      browser("press", "Escape"); // Dismiss the tooltip without removing focus.
      screenshot(`sidebar-${width}-keyboard`);
      browser("click", ".direct-self");
      wait(
        `document.querySelector('.direct-self[aria-current="page"]') && document.querySelector('#chat-message:not(:disabled)')`,
      );
      if (width === 390) click("Back to Browse");
      assert.deepEqual(channelGeometry(), before, "Opening a DM must not shrink channel rows");
      assert.equal(
        evaluate(`return getComputedStyle(document.querySelector('.direct-self')).outlineStyle;`),
        "none",
        "Clicking a DM never draws a ring",
      );
      assert.equal(
        evaluate(`return document.querySelectorAll('.channel-join').length;`),
        3,
        "DM navigation retains every channel's join shortcut",
      );
      browser("mouse", "move", "5", "5");
      screenshot(`sidebar-${width}-dm`);
      if (desktopHover()) {
        browser("hover", '.channel-line:has([aria-label="Join voice in #beta"])');
        click("Join voice in #beta");
      } else {
        // Touch shows quiet rows' shortcut at rest; its handler is the same.
        evaluate(`document.querySelector('[aria-label="Join voice in #beta"]').click();`);
      }
      wait(`voiceFixture.client?.phase === 'connected' && document.querySelector('[aria-label="Leave voice"]')`);
      assert.equal(
        evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).channelId;`),
        "beta00000000",
        "Join from a DM targets the chosen space channel",
      );
      if (width === 390) {
        click("Close navigation");
        click("Back to Browse");
      }
      chooseChannel("alpha");
      wait(`document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha'`);
      if (width === 390) click("Back to Browse");
      const leaves = evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`);
      browser("click", ".direct-self");
      wait(`document.querySelector('.direct-self[aria-current="page"]')`);
      assert.equal(
        evaluate(`return voiceFixture.client.phase;`),
        "connected",
        "Opening a DM must preserve an active voice connection",
      );
      assert.equal(evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`), leaves);
      if (width === 390) {
        // Phones read the DM under the call's strip; tapping it opens Browse, where Leave is.
        assert.ok(evaluate(`return !!document.querySelector('.voice-strip .voice-strip-open');`));
        browser("click", ".voice-strip .voice-strip-open");
        settleBrowse();
        wait(`document.querySelector('.navigation-open .live-call')`);
      }
      click("Leave voice");
      wait(`voiceFixture.client.phase === 'idle'`);
    }
    browser("set", "media", "no-preference");
    // A space without a joined channel and a rail invitation replace only the
    // conversation: the room, and a call running elsewhere, stay mounted.
    browser("set", "viewport", "1280", "900", "2");
    evaluate(`voiceFixture.invitations = [{ id: "invited00000", name: "Invited space", ownerId: "other-owner",
      inviter: { id: "inviter", username: "inviter", displayName: "Inviter" } }];
      voiceFixture.showSpaces(true);`);
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    const mediaLeaves = () => evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`);
    click("Other test space");
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'lobby' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    click("Join voice in #lobby");
    wait(`voiceFixture.client?.phase === 'connected' && document.querySelector('[aria-label="Leave voice"]')`);
    let leaves = mediaLeaves();
    evaluate(`voiceFixture.otherDeleted = true; window.dispatchEvent(new Event("focus"));`);
    wait(`document.querySelector('.empty-channel') && voiceFixture.client.phase === 'idle'`);
    assert.equal(mediaLeaves(), leaves + 1, "A call whose channel disappears still ends in the empty view");
    click("Test space");
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    click("Join voice in #alpha");
    wait(`voiceFixture.client?.phase === 'connected' && document.querySelector('[aria-label="Leave voice"]')`);
    leaves = mediaLeaves();
    click("Other test space");
    wait(`document.querySelector('.empty-channel')`);
    assert.equal(
      evaluate(`return voiceFixture.client.phase;`),
      "connected",
      "A space with no joined channel keeps the call",
    );
    assert.equal(mediaLeaves(), leaves);
    assert.equal(
      evaluate(
        `const card = document.querySelector('.live-call'); return card && card.querySelector('.live-channel').textContent + '|' + !!card.querySelector('[aria-label="Leave voice"]');`,
      ),
      "alphaTest space|true",
      "Your call card names the call's channel and space, with Leave",
    );
    screenshot("empty-space-in-call-1280");
    browser("set", "viewport", "390", "844", "2");
    wait(`document.querySelector('.empty-channel')?.getBoundingClientRect().width < 390`);
    assert.equal(evaluate(`return document.documentElement.scrollWidth > innerWidth;`), false);
    // Phones show the empty space alone; Browse keeps your call card and Leave.
    openBrowse();
    assert.ok(
      evaluate(`const r = document.querySelector('.live-call [aria-label="Leave voice"]').getBoundingClientRect();
        return r.width > 0 && r.top >= 0 && r.bottom <= innerHeight;`),
      "Phones reach the call's Leave from Browse",
    );
    screenshot("empty-space-in-call-390");
    browser("set", "viewport", "1280", "900", "2");
    click("Invitation to Invited space");
    wait(`document.querySelector('.invitation-consent')`);
    assert.equal(evaluate(`return voiceFixture.client.phase;`), "connected", "A rail invitation keeps the call");
    assert.ok(evaluate(`return !!document.querySelector('.empty-channel');`), "The invitation opens over the room");
    screenshot("rail-invitation-in-call-1280");
    browser("press", "Escape");
    wait(`!document.querySelector('.invitation-consent')`);
    assert.equal(evaluate(`return voiceFixture.client.phase;`), "connected");
    assert.equal(mediaLeaves(), leaves);
    click("Leave voice");
    wait(`voiceFixture.client.phase === 'idle'`);
    evaluate(`voiceFixture.invitations = []; voiceFixture.otherDeleted = false;`);
    // A cold DM URL must check a real channel, not the DM's media endpoint.
    browser("set", "viewport", "1280", "900", "2");
    evaluate(`voiceFixture.showSpaces(true, 'selfdm000000');`);
    wait(
      `document.querySelector('.direct-self[aria-current="page"]') && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    assert.equal(
      evaluate(
        `return voiceFixture.commands.filter(c => c.method.startsWith('media.') && c.channelId === 'selfdm000000').length;`,
      ),
      0,
      "A DM never provisions or subscribes to its own voice",
    );
    evaluate(`voiceFixture.noSpaces = true; voiceFixture.showSpaces(false, 'selfdm000000');`);
    wait(`document.querySelector('.direct-self[aria-current="page"]') && !document.querySelector('.channel-line')`);
    assert.equal(
      evaluate(`return document.querySelectorAll('.channel-join').length;`),
      0,
      "An account without spaces exposes no DM voice action",
    );
    evaluate(`await voiceFixture.cleanup();`);
    console.log(
      "PASS DM row geometry, voice retention/join from DMs, calls kept across empty spaces and rail invitations, cold DM/no-space routes, hover/open/keyboard menus, ringless click/keyboard focus and 6px channel gaps at 1280px/390px (mock signaling/WebRTC)",
    );
  } else if (process.env.VOICE_TEST_REDESIGN === "1") {
    // Live channels: a live row grows into its card in the list (your call, or a room
    // with Join), one-click joins from rooms and rows, no voice in the conversation
    // header, and a strip under it on phones with the card's faces.
    evaluate(`voiceFixture.timerEnabled = true; voiceFixture.showSpaces();`);
    wait(`document.querySelector('[aria-label="Join voice in #alpha"]')?.getAttribute('aria-disabled') === 'false'`);
    const footerControls = () => {
      for (const label of ["Input Options", "Output Options", "User Settings"]) {
        assert.equal(evaluate(`return document.querySelectorAll('[aria-label="${label}"]').length;`), 1);
      }
      assert.equal(evaluate(`return document.querySelectorAll('.call-account .voice-action-group').length;`), 2);
    };
    const current = (name) =>
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === '${name}'`;
    // Phones: close Browse onto the conversation the current row opens.
    const readConversation = () => {
      browser("click", '.channel-select[aria-current="page"]');
      settleBrowse();
      wait(`!document.querySelector('.navigation-open')`);
    };
    const lastJoin = () =>
      evaluate(`return voiceFixture.commands.filter(command => command.method === 'media.join').at(-1).channelId;`);
    for (const width of [1280, 390]) {
      const phone = width === 390;
      browser("set", "viewport", String(width), "900", "2");
      if (phone) openBrowse();
      // Each pass starts in #alpha, where Maya and Alex are talking.
      if (!evaluate(`return ${current("alpha")};`)) {
        browser("click", "#space-channel-list > li:first-child .channel-select");
        wait(current("alpha"));
        if (phone) openBrowse();
      }
      evaluate(`voiceFixture.sessionStartedAt = Date.now() - 37000; voiceFixture.publishPresence([
        {id:'peer',name:'Maya',avatarId:31,muted:false,deafened:false,tracks:[]},
        {id:'other',name:'Alex',avatarId:799,muted:true,deafened:false,tracks:[]}
      ]);`);
      wait(`document.querySelector('.live-room')?.getAttribute('aria-label') === 'Voice in #alpha, 2 in voice'`);
      browser("mouse", "move", "5", "5");
      footerControls();
      assert.deepEqual(
        evaluate(`const room = document.querySelector('.live-room'), join = room.querySelector('.live-join');
          return [!!room.closest('#space-channel-list > li'), room.querySelector('.channel-select > span').textContent,
            room.querySelector('.live-names').textContent, room.querySelectorAll('.live-stack .live-avatar').length,
            !!room.querySelector('.live-room-body [role="timer"]'), join.textContent, getComputedStyle(join).backgroundColor];`),
        [true, "alpha", "Maya and Alex", 2, true, "Join", "rgb(241, 237, 228)"],
        "A live room's own row grows into its card: its people, timer and an eggshell Join",
      );
      assert.equal(
        evaluate(`return document.querySelectorAll('.live-now, .live-now-heading, .live-more-rooms').length
          + [...document.querySelectorAll('.people-panel *')].filter(node => !node.children.length && node.textContent.trim() === 'Live now').length;`),
        0,
        "No Live now heading, fold or separate section: rooms stay in the list",
      );
      assert.equal(
        evaluate(
          `return [...document.querySelectorAll('.channel-line')].filter(line => line.querySelector('.channel-live')).map(line => line.querySelector('.channel-select > span').textContent + ':' + line.querySelector('.channel-live').getAttribute('aria-label')).join();`,
        ),
        "alpha:2 in voice",
        "Only occupied rows carry the live mark",
      );
      assert.equal(
        evaluate(`return document.querySelectorAll('.participant-menu-button, .live-call, .live-everyone').length;`),
        0,
        "Spectators have no call card or audio controls",
      );
      screenshot(`voice-live-browse-${width}`);
      if (phone) readConversation();
      assert.equal(
        evaluate(
          `return !!document.querySelector('.chat-heading :is(.channel-join, .voice-hangup, .live-toggle, .live-face, .header-voice)');`,
        ),
        false,
        "The conversation header carries no voice controls",
      );
      assert.match(evaluate(`return document.querySelector('.chat-title-subtitle')?.textContent ?? '';`), /2 in voice/);
      if (phone) {
        assert.deepEqual(
          evaluate(`const strip = document.querySelector('.voice-strip');
            return strip && [strip.querySelector('strong').textContent, strip.querySelector('.voice-strip-detail').textContent,
              strip.querySelector('.live-join').getAttribute('aria-label')];`),
          ["2 talking", "Maya and Alex", "Join voice in #alpha"],
          "Phones show the open channel's live room under the header",
        );
        assert.ok(
          evaluate(`const strip = document.querySelector('.voice-strip').getBoundingClientRect(), header = document.querySelector('.chat-heading').getBoundingClientRect();
            return Math.abs(strip.top - header.bottom) <= 1 && strip.height === 56;`),
          "The strip sits directly under the header",
        );
        screenshot(`voice-live-strip-idle-${width}`);
        openBrowse();
      } else {
        assert.equal(evaluate(`return !!document.querySelector('.voice-strip');`), false, "Wide layouts have no strip");
      }

      click("Input Options");
      browser("select", 'select[name="input-device"]', "headset");
      browser("focus", '[aria-label="Input volume"]');
      browser("press", "Home");
      browser("press", "PageUp");
      assert.equal(
        evaluate(`return document.querySelector('[aria-label="Input volume"]').getAttribute('aria-valuenow');`),
        "10",
      );
      browser("press", "Escape");
      click("Output Options");
      browser("select", 'select[name="output-device"]', "headphones");
      browser("focus", '[aria-label="Output volume"]');
      browser("press", "End");
      browser("press", "PageDown");
      assert.equal(
        evaluate(`return document.querySelector('[aria-label="Output volume"]').getAttribute('aria-valuenow');`),
        "190",
      );
      browser("press", "Escape");
      click("Mute microphone");
      click("Deafen audio");
      const captures = evaluate(`return voiceFixture.captures.length;`);
      // Desktop joins from the room card; phones from the strip, staying in the conversation.
      if (phone) {
        readConversation();
        browser("focus", ".voice-strip .live-join");
      } else browser("focus", ".live-room .live-join");
      browser("press", "Enter");
      wait(`voiceFixture.client?.phase === 'connected' && document.querySelector('.live-call')`);
      assert.equal(lastJoin(), "alpha0000000");
      assert.equal(evaluate(`return voiceFixture.captures.length;`), captures + 1);
      assert.ok(
        evaluate(
          phone
            ? `return document.activeElement.matches('.voice-strip .voice-strip-open');`
            : `return document.activeElement.matches('.live-call .voice-hangup');`,
        ),
        "Join hands keyboard focus to your call (its strip on phones, Leave on its card)",
      );
      if (phone) {
        assert.deepEqual(
          evaluate(`const strip = document.querySelector('.voice-strip');
            return [strip.querySelector('strong').textContent, !!strip.querySelector('[role="timer"]'),
              strip.querySelector('.live-toggle').getAttribute('aria-label'), strip.querySelectorAll('.live-avatar').length,
              strip.querySelector('.voice-strip-detail').textContent, !!document.querySelector('.navigation-open')];`),
          ["#alpha", true, "Unmute microphone", 3, "", false],
          "Joining keeps the conversation, under your call's strip: everyone's faces, no speaker's name",
        );
        assert.deepEqual(
          evaluate(`return ${faceIds(".voice-strip .live-avatar")};`),
          evaluate(`return ${faceIds(".live-call .live-face .live-avatar")};`),
          "The strip shows the same people as the call card, in its order",
        );
        screenshot(`voice-live-strip-connected-${width}`);
        // Tapping the strip opens Browse, where your call card is.
        browser("click", ".voice-strip .voice-strip-open");
        settleBrowse();
        wait(`document.querySelector('.navigation-open .live-call')`);
      }
      assert.deepEqual(
        evaluate(`const card = document.querySelector('.live-call');
          return [!!card.closest('#space-channel-list > li'), card.querySelector('.channel-select > span').textContent,
            card.querySelector('.live-people-count').textContent.trim(), !!card.querySelector('[role="timer"]'),
            [...card.querySelectorAll('.live-face')].map(face => face.getAttribute('aria-label')),
            document.querySelectorAll('.live-room').length, document.querySelectorAll('[aria-label="Leave voice"]').length,
            [...document.querySelectorAll('.people-panel .channel-select > span')].filter(name => name.textContent === 'alpha').length,
            card.querySelectorAll('button.live-face, button.live-more, .live-speaker').length];`),
        [true, "alpha", "3 people", true, ["UI fixture (you), deafened", "Maya", "Alex, muted"], 0, 1, 1, 0],
        "Your call's row is its card: channel, count, timer, you first with badges, one Leave, no duplicate row or speaker line",
      );
      assert.ok(
        evaluate(`const card = document.querySelector('.live-call'), count = card.querySelector('.live-people-count');
          return !card.querySelector('.live-status') && !card.textContent.includes('You’re in') &&
            count.getBoundingClientRect().top >= card.querySelector('.live-faces').getBoundingClientRect().bottom &&
            card.querySelector('[role="timer"]').getBoundingClientRect().bottom < count.getBoundingClientRect().top;`),
        "The connected heading keeps only the timer; a labelled count sits with the faces below it",
      );
      assert.equal(evaluate(`return voiceFixture.devices.at(-1);`), "headset");
      assert.deepEqual(
        evaluate(`return [voiceFixture.client.muted, voiceFixture.client.deafened, voiceFixture.client.inputVolume];`),
        [true, true, 10],
      );
      footerControls();
      evaluate(`document.activeElement.blur();`);
      browser("mouse", "move", "5", "5");
      screenshot(`voice-live-connected-${width}`);
      // A right-click (touch: a long press) on a face opens the full list with that person's
      // audio controls; the press does not also open the chat.
      const readingBefore = evaluate(
        `return document.querySelector('.channel-select[aria-current="page"] > span').textContent;`,
      );
      if (phone) longPress('.live-call .live-face[aria-label="Alex, muted"]');
      else rightClick('.live-call .live-face[aria-label="Alex, muted"]');
      wait(`document.activeElement === document.querySelector('.live-everyone [aria-label="Alex volume"]')`);
      assert.ok(
        evaluate(`return ${current(readingBefore)};`),
        "Opening everyone's volume keeps the chat you are reading",
      );
      // Measure once the list's short entrance has settled.
      wait(`!document.querySelector('.live-everyone').getAnimations().some(a => a.playState === 'running')`);
      assert.ok(
        evaluate(`const list = document.querySelector('.live-everyone').getBoundingClientRect(), menu = document.querySelector('.participant-volume'), r = menu.getBoundingClientRect();
          return list.top >= 0 && list.bottom <= innerHeight + 1 && r.top >= 0 && r.bottom <= innerHeight && r.left >= 0 && r.right <= innerWidth
            && menu.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2))
            && ${phone ? "Math.abs(list.bottom - innerHeight) <= 1" : "list.left >= document.querySelector('.people-panel').getBoundingClientRect().right - 1"};`),
        phone ? "Phones open the list as a bottom sheet" : "The list opens beside the sidebar",
      );
      assert.equal(
        evaluate(`return document.querySelector('.live-everyone-heading p').textContent;`),
        "In #alpha3 people",
      );
      screenshot(`voice-live-everyone-${width}`);
      browser("press", "Escape");
      wait(`!document.querySelector('.participant-volume') && document.querySelector('.live-everyone:popover-open')`);
      browser("press", "Escape");
      wait(`!document.querySelector('.live-everyone')`);
      evaluate(`voiceFixture.joinedTrack = voiceFixture.captures.at(-1);`);

      if (!phone) {
        browser("click", '[aria-controls="space-channel-list"]');
        assert.ok(
          evaluate(
            `const leave = document.querySelector('[aria-label="Leave voice"]'), r = leave.getBoundingClientRect(); return leave.checkVisibility() && r.width > 0 && r.height > 0;`,
          ),
          "Collapsing channels must keep the active call reachable",
        );
        assert.deepEqual(
          evaluate(
            `return [...document.querySelectorAll('#space-channel-list > li')].filter(li => li.checkVisibility()).map(li => li.querySelector('.channel-select > span').textContent);`,
          ),
          ["alpha"],
          "Collapsed channels keep only the live ones",
        );
        browser("click", '[aria-controls="space-channel-list"]');
      }
      chooseChannel("beta");
      wait(current("beta"));
      assert.equal(evaluate(`return voiceFixture.joinedTrack.readyState;`), "live");
      if (phone) {
        assert.equal(
          evaluate(`return document.querySelector('.voice-strip strong')?.textContent;`),
          "#alpha",
          "Reading another channel keeps your call's strip",
        );
      } else {
        assert.ok(
          evaluate(
            `return ${connectedRow("alpha")} && !!document.querySelector('[aria-label="Switch voice to #beta"]');`,
          ),
          "Reading another channel keeps your call card; its row offers Switch",
        );
      }
      screenshot(`voice-live-browsing-${width}`);
      // Pressing the card anywhere, here on a face, opens the call's chat.
      if (phone) openBrowse();
      browser("click", '.live-call .live-face[aria-label="Maya"]');
      wait(current("alpha"));
      assert.equal(
        evaluate(`return voiceFixture.client.phase;`),
        "connected",
        "Opening the call's chat keeps the call",
      );
      assert.equal(
        evaluate(`return !!document.querySelector('.live-everyone');`),
        false,
        "A plain press opens no list",
      );
      if (phone) openBrowse();
      click("Other test space");
      wait(current("lobby"));
      if (phone) {
        assert.equal(evaluate(`return document.querySelector('.voice-strip strong')?.textContent;`), "#alpha");
        openBrowse();
      }
      assert.equal(
        evaluate(`return document.querySelector('.live-now > .live-call .live-channel')?.textContent;`),
        "alphaTest space",
        "Elsewhere, your call card sits above the channels and names its space",
      );
      assert.equal(
        evaluate(`return voiceFixture.joinedTrack.readyState;`),
        "live",
        "Switching spaces does not leave voice",
      );
      assert.equal(evaluate(`return document.querySelectorAll('[aria-label="Leave voice"]').length;`), 1);
      footerControls();
      screenshot(`voice-live-other-space-${width}`);
      click("Leave voice");
      wait(`voiceFixture.client.phase === 'idle' && !document.querySelector('.live-call')`);
      assert.equal(evaluate(`return voiceFixture.joinedTrack.readyState;`), "ended");
      footerControls();
      click("Undeafen audio");
      if (evaluate(`return !!document.querySelector('[aria-label="Unmute microphone"]');`)) click("Unmute microphone");
      click("Test space");
      wait(`document.querySelector('.space-menu h1')?.textContent === 'Test space'`);
      if (phone) openBrowse();
      // A row joins in one click without opening that channel; another row switches.
      const reading = evaluate(
        `return document.querySelector('.channel-select[aria-current="page"] > span')?.textContent;`,
      );
      browser("click", '.channel-line [aria-label="Join voice in #alpha"]');
      wait(`voiceFixture.client.phase === 'connected' && ${connectedRow("alpha")}`);
      browser("click", '.channel-line [aria-label="Switch voice to #beta"]');
      // (Timed fixtures report people only in #alpha, so #beta's row has no live mark.)
      wait(`voiceFixture.client.phase === 'connected' && ${connectedRow("beta")}`);
      assert.equal(lastJoin(), "beta00000000");
      assert.ok(evaluate(`return ${current(reading)};`), "Joining from a row keeps the conversation you are reading");
      click("Leave voice");
      wait(`voiceFixture.client.phase === 'idle'`);
    }
    // Busy spaces: every live room grows from its own row, in list order; nothing folds away.
    browser("set", "viewport", "1280", "900", "2");
    evaluate(`voiceFixture.timerEnabled = false; voiceFixture.publishPresence([
      {id:'peer',name:'Maya',avatarId:31,muted:false,deafened:false,tracks:[]},
      {id:'other',name:'Alex',avatarId:799,muted:true,deafened:false,tracks:[]}
    ]);`);
    wait(`document.querySelectorAll('.live-room').length === 3`);
    assert.deepEqual(
      evaluate(
        `return [[...document.querySelectorAll('#space-channel-list > li > .live-room .channel-select > span:first-of-type')].map(node => node.textContent), document.querySelectorAll('.live-now, .live-more-rooms').length];`,
      ),
      [["alpha", "beta", "planning-for-the-next-release"], 0],
      "Live rooms stay in the list's order",
    );
    screenshot("voice-live-rooms-1280");
    // Crowded calls: nine faces and a +N tile whose right-click opens everyone.
    browser("click", '.live-room .live-join[aria-label="Join voice in #alpha"]');
    wait(`voiceFixture.client.phase === 'connected' && document.querySelector('.live-call')`);
    evaluate(`const client = voiceFixture.client;
      client.participants = [client.participants.find(person => person.id === 'self'), ...Array.from({ length: 12 }, (_, i) =>
        ({ id: 'crowd' + i, name: 'Crowd ' + (i + 1), avatarId: i + 1, muted: false, deafened: false, tracks: [] }))];
      client.emit();`);
    wait(`document.querySelectorAll('.live-call .live-face').length === 9`);
    assert.deepEqual(
      evaluate(`const more = document.querySelector('.live-call .live-more'), tops = new Set([...document.querySelectorAll('.live-call .live-faces > li')].map(li => Math.round(li.getBoundingClientRect().top)));
        return [more.textContent, more.getAttribute('aria-label'), tops.size, document.querySelector('.live-call .live-people-count').textContent.trim()];`),
      ["+4", "4 more: Crowd 9, Crowd 10 and 2 more", 2, "13 people"],
      "Thirteen people fit two rows: nine faces and +4",
    );
    rightClick(".live-call .live-more");
    wait(`document.querySelectorAll('.live-everyone .participant').length === 13`);
    assert.ok(
      evaluate(
        `const list = document.querySelector('.live-everyone'), r = list.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight && list.querySelector('ul').scrollHeight >= list.querySelector('ul').clientHeight;`,
      ),
      "The full list stays within the viewport",
    );
    screenshot("voice-live-crowd-1280");
    browser("press", "Escape");
    wait(`!document.querySelector('.live-everyone')`);
    // The phone strip: the card's first four faces, in its order, then +N for the rest.
    browser("set", "viewport", "390", "900", "2");
    wait(`document.querySelector('.voice-strip .live-chip')`);
    assert.deepEqual(
      evaluate(`const strip = ${faceIds(".voice-strip .live-avatar")}, card = ${faceIds(".live-call .live-face .live-avatar")};
        return [strip.length, strip.join() === card.slice(0, 4).join(), document.querySelector('.voice-strip .live-chip').textContent];`),
      [4, true, "+9"],
      "A crowded call's strip folds after four faces, keeping the card's order",
    );
    browser("set", "viewport", "1280", "900", "2");
    click("Leave voice");
    wait(`voiceFixture.client.phase === 'idle'`);
    evaluate(`await voiceFixture.cleanup();`);
    console.log(
      "PASS live channels at 1280px/390px: rooms and your call as cards in their rows (no Live now heading, fold or duplicate row), voice-free header, phone strip join/focus/Browse with the card's faces, pre-join device/volume/mute/deafen, call card faces/badges/order, card press opens chat, right-click/long press opens the full list, collapsed navigation keeps live rows, chat/space persistence, row join/switch and crowded calls (mock API/WebRTC).",
    );
  } else if (process.env.VOICE_TEST_DOCK === "1") {
    evaluate(`voiceFixture.showSpaces();`);
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    click("Mute microphone");
    click("Join voice in #alpha");
    wait(`document.querySelector('[aria-label="Leave voice"]')`);
    for (const width of [1280, 390]) {
      browser("set", "viewport", String(width), "900", "2");
      if (width === 390) openBrowse();
      click("Other test space");
      wait(`document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'lobby'`);
      if (width === 390) openBrowse();
      evaluate(
        `await document.fonts.ready; await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`,
      );
      assert.ok(
        evaluate(`const dock = document.querySelector('.call-account').getBoundingClientRect();
        return [...document.querySelectorAll('.call-account :is(.account-avatar, .account-name, .voice-icon-button, .call-settings-trigger)')].every(node => {
          const box = node.getBoundingClientRect();
          return Math.abs(box.top + box.height / 2 - (dock.top + dock.height / 2)) < .5 && box.top >= dock.top && box.bottom <= dock.bottom;
        });`),
        "Connected profile/audio controls are centered and contained, not clipped",
      );
      assert.equal(
        evaluate(`return document.querySelectorAll('.voice-dock .connected-channel, .voice-pinned').length;`),
        0,
        "Your call lives on its card; no dock or pinned roster",
      );
      assert.ok(
        evaluate(
          `const card = document.querySelector('.live-call').getBoundingClientRect(), heading = document.querySelector('.channel-section-heading').getBoundingClientRect(); return card.bottom <= heading.top;`,
        ),
        "Your call card sits above the other space's channels",
      );
      browser("mouse", "move", "5", "5");
      screenshot(`dock-connected-${width}`);
      // The card's channel name opens the call's chat, back in its own space.
      browser("click", ".live-call .live-channel");
      wait(
        `document.querySelector('.space-menu h1')?.textContent === 'Test space' && document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha'`,
      );
      assert.equal(
        evaluate(`return voiceFixture.client.phase;`),
        "connected",
        "Opening the call's chat keeps the call",
      );
    }
    if (!evaluate(`return !!document.querySelector('.navigation-open');`) && evaluate(`return innerWidth <= 760;`))
      openBrowse();
    click("Leave voice");
    wait(`!document.querySelector('.live-call')`);
    assert.equal(
      evaluate(`return voiceFixture.captures.at(-1).readyState;`),
      "ended",
      "Disconnect still releases the microphone",
    );
    evaluate(`await voiceFixture.cleanup();`);
    console.log(
      "PASS centered/unclipped connected account bar, call card above another space's channels, card link back to the call's chat, and working disconnect at 1280px/390px (mock signaling/WebRTC)",
    );
  } else if (process.env.VOICE_TEST_TIMERS === "1") {
    evaluate(`voiceFixture.timerEnabled = true; voiceFixture.showSpaces();`);
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    // Your call card's timer while you join or talk; #alpha's room card's while you watch.
    const timer = '.live-card [role="timer"]';
    for (const width of [1280, 390]) {
      browser("set", "viewport", String(width), "900", "2");
      if (width === 390) openBrowse();
      evaluate(
        `voiceFixture.holdJoin = true; voiceFixture.releaseJoin = undefined; voiceFixture.clicked = Date.now();`,
      );
      click("Join voice in #alpha");
      wait(`voiceFixture.releaseJoin && document.querySelector(${JSON.stringify(timer)})`);
      assert.ok(
        evaluate(
          `return Date.now() - voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).body.joinStartedAt < 1_000;`,
        ),
        "The join request carries the click time",
      );
      browser("wait", "2300");
      assert.ok(
        evaluate(
          `return /^00:0[2-9]$/.test(document.querySelector(${JSON.stringify(timer)}).textContent) && voiceFixture.client.phase === 'joining';`,
        ),
        "The timer counts while the server join is still pending",
      );
      assert.equal(
        evaluate(`return document.querySelectorAll('.channel-live, .live-room, .live-call .live-face').length;`),
        0,
        "A pending timer cannot invent occupancy",
      );
      evaluate(`await document.fonts.ready;`);
      screenshot(`voice-timer-${width}-joining`);
      evaluate(`voiceFixture.holdJoin = false; voiceFixture.releaseJoin();`);
      wait(`document.querySelector('[aria-label="Leave voice"]')`);
      assert.ok(
        evaluate(
          `return voiceFixture.client.sessionStartedAt === voiceFixture.sessionStartedAt && voiceFixture.sessionStartedAt === voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).body.joinStartedAt;`,
        ),
        "The connected timer adopts the shared click timestamp without restarting",
      );
      click("Leave voice");
      wait(
        `document.querySelector('[aria-label="Join voice in #alpha"]') && !document.querySelector(${JSON.stringify(timer)})`,
      );
      evaluate(
        `voiceFixture.sessionStartedAt = Date.now() - 601_000; voiceFixture.publishPresence([{ id: 'peer', name: 'Timer test observer', muted: false, deafened: false, tracks: [] }]);`,
      );
      wait(`/^10:0[1-9]$/.test(document.querySelector(${JSON.stringify(timer)})?.textContent)`);
      screenshot(`voice-timer-${width}-spectator`);
      evaluate(`voiceFixture.holdJoin = true; voiceFixture.releaseJoin = undefined;`);
      click("Join voice in #alpha");
      wait(
        `voiceFixture.releaseJoin && voiceFixture.client.phase === 'joining' && document.querySelector('[aria-label="Cancel joining voice"]')`,
      );
      assert.ok(
        evaluate(`return document.querySelector(${JSON.stringify(timer)}).textContent.startsWith('10:');`),
        "Joining an occupied room preserves its existing duration",
      );
      click("Cancel joining voice");
      wait(`document.querySelector('[aria-label="Join voice in #alpha"]')`);
      evaluate(`voiceFixture.holdJoin = false; voiceFixture.releaseJoin();`);
      wait(`voiceFixture.commands.at(-1).method === 'media.leave'`);
      assert.ok(
        evaluate(
          `return document.querySelector(${JSON.stringify(timer)}).textContent.startsWith('10:') && voiceFixture.people.length === 1;`,
        ),
        "Cancelling cannot clear another participant's session timer",
      );
      evaluate(`voiceFixture.sessionStartedAt = undefined; voiceFixture.publishPresence([]);`);
      wait(`!document.querySelector(${JSON.stringify(timer)})`);
      evaluate(`voiceFixture.holdJoin = true; voiceFixture.releaseJoin = undefined;`);
      click("Join voice in #alpha");
      wait(`voiceFixture.releaseJoin && document.querySelector('[aria-label="Cancel joining voice"]')`);
      evaluate(
        `voiceFixture.sessionStartedAt = Date.now() - 601_000; voiceFixture.publishPresence([{ id: 'peer', name: 'Timer test observer', muted: false, deafened: false, tracks: [] }]);`,
      );
      wait(`/^10:0[1-9]$/.test(document.querySelector(${JSON.stringify(timer)})?.textContent)`);
      assert.equal(
        evaluate(`return voiceFixture.client.phase;`),
        "joining",
        "A late spectator snapshot corrects provisional time before connection completes",
      );
      click("Cancel joining voice");
      evaluate(`voiceFixture.holdJoin = false; voiceFixture.releaseJoin();`);
      wait(`voiceFixture.commands.at(-1).method === 'media.leave'`);
      evaluate(`voiceFixture.sessionStartedAt = undefined; voiceFixture.publishPresence([]);`);
      wait(
        `document.querySelector('[aria-label="Join voice in #alpha"]') && !document.querySelector(${JSON.stringify(timer)})`,
      );
      evaluate(`voiceFixture.captureError = 'NotAllowedError';`);
      click("Join voice in #alpha");
      wait(`voiceFixture.client.phase === 'failed' && !document.querySelector(${JSON.stringify(timer)})`);
      evaluate(`voiceFixture.captureError = undefined;`);
    }
    evaluate(`await voiceFixture.cleanup();`);
    console.log(
      "PASS click-time timer during delayed join, shared spectator duration, occupied-room retention, cancel/failure cleanup at 1280px and 390px (mock signaling/WebRTC)",
    );
  } else if (process.env.VOICE_TEST_AVATARS === "1") {
    evaluate(`voiceFixture.showSpaces();`);
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    click("Mute microphone");
    click("Join voice in #alpha");
    wait(`document.querySelector('[aria-label="Leave voice"]')`);
    evaluate(`voiceFixture.addRemoteAudio(); await document.fonts.ready;`);
    wait(`document.querySelectorAll('.live-call .live-avatar').length === 2`);
    for (const width of [1280, 390]) {
      browser("set", "viewport", String(width), "900", "2");
      if (width === 390) openBrowse();
      evaluate(`await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`);
      // Card faces: borderless at rest, a green ring (with a gap) while speaking.
      assert.deepEqual(
        evaluate(
          `return [...document.querySelectorAll('.live-call .live-avatar')].map(el => { const s = getComputedStyle(el); return [s.borderTopWidth, s.boxShadow, s.width, s.height]; });`,
        ),
        Array(2).fill(["0px", "none", "34px", "34px"]),
        "Idle image and initial faces have no ring, without resizing",
      );
      assert.deepEqual(
        evaluate(
          `return [...document.querySelectorAll('.live-call .live-avatar > span')].map(el => el.getAttribute('data-avatar-id'));`,
        ),
        ["0", null],
        "Exercise both image artwork and the initials fallback",
      );
      rightClick(".live-call .live-face");
      wait(`document.querySelectorAll('.live-everyone .avatar.quiet').length === 2`);
      assert.deepEqual(
        evaluate(
          `return [...document.querySelectorAll('.live-everyone .avatar')].map(el => { const s = getComputedStyle(el); return [s.borderTopColor, s.boxShadow]; });`,
        ),
        Array(2).fill(["rgba(0, 0, 0, 0)", "none"]),
        "The full list's idle avatars have no visible border or halo",
      );
      const nameStyles = () =>
        evaluate(
          `return [...document.querySelectorAll('.live-everyone .participant-name strong')].map(el => { const s = getComputedStyle(el); return [s.color, s.fontWeight]; });`,
        );
      const quietNames = Array(2).fill(["rgb(185, 188, 190)", "500"]);
      assert.deepEqual(nameStyles(), quietNames, "Quiet participant names step back");
      screenshot(`voice-names-quiet-${width}`);
      browser("press", "Escape");
      wait(`!document.querySelector('.live-everyone')`);
      browser("mouse", "move", "5", "5");
      screenshot(`voice-avatars-idle-${width}`);
      // Pauses must not resize the card and move the channels below it.
      const cardHeight = () => evaluate(`return document.querySelector('.live-call').getBoundingClientRect().height;`);
      const quietHeight = cardHeight();
      click("Unmute microphone");
      wait(`document.querySelector('.live-call .live-avatar[data-speaking]')`);
      assert.equal(cardHeight(), quietHeight, "Someone starting to speak keeps the call card's height");
      assert.match(
        evaluate(
          `return getComputedStyle(document.querySelector('.live-call .live-avatar[data-speaking]')).boxShadow;`,
        ),
        /rgb\(74, 168, 107\) 0px 0px 0px 4px/,
        "Speaking shows the voice-green ring",
      );
      assert.equal(
        evaluate(`return document.querySelector('.live-call').textContent.includes('speaking');`),
        false,
        "Speaking only rings the face: there is no speaker line",
      );
      if (width === 390) {
        // The phone strip over the conversation rings its speaking face too.
        browser("click", '.channel-select[aria-current="page"]');
        settleBrowse();
        wait(`document.querySelector('.voice-strip .live-avatar[data-speaking]')`);
        assert.match(
          evaluate(
            `return getComputedStyle(document.querySelector('.voice-strip .live-avatar[data-speaking]')).boxShadow;`,
          ),
          /rgb\(74, 168, 107\) 0px 0px 0px 4px/,
          "The strip's speaking face shows the voice-green ring",
        );
        screenshot("voice-avatars-strip-speaking-390");
        openBrowse();
      }
      browser("mouse", "move", "5", "5");
      screenshot(`voice-avatars-speaking-${width}`);
      rightClick(".live-call .live-face");
      wait(`document.querySelector('.live-everyone .avatar.speaking')`);
      assert.deepEqual(
        nameStyles(),
        [["rgb(243, 244, 245)", "900"], quietNames[1]],
        "Only the speaking participant's name becomes bright and bold",
      );
      screenshot(`voice-names-speaking-${width}`);
      // Keep the list open: ending speech must update the existing name too.
      evaluate(`await voiceFixture.client.setMuted(true);`);
      wait(`!document.querySelector('.live-everyone .avatar.speaking')`);
      assert.deepEqual(nameStyles(), quietNames, "Names return to quiet styling when muted");
      browser("press", "Escape");
      wait(`!document.querySelector('.live-everyone')`);
      wait(`!document.querySelector('.live-call .live-avatar[data-speaking]')`);
      assert.equal(cardHeight(), quietHeight, "Falling quiet keeps the call card's height");
    }
    evaluate(`await voiceFixture.cleanup();`);
    console.log(
      "PASS idle/speaking avatar rings and bright/bold speaker names with quiet/mute reset at 1280px and 390px (mock signaling/WebRTC)",
    );
  } else if (process.env.VOICE_TEST_CHANNEL_ROWS === "1") {
    evaluate(`voiceFixture.showSpaces();`);
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    evaluate(`await document.fonts.ready;`);
    const geometry = () =>
      evaluate(
        // Within each row (a live card above may move whole rows), plus the row's own size.
        `return [...document.querySelectorAll('.channel-line')].map(line => { const top = line.getBoundingClientRect().top; return [...line.querySelectorAll('.channel-select, .channel-manage, .channel-quick-join'), line].map(node => { const r = node.getBoundingClientRect(); return [r.x, r.y - top, r.width, r.height]; }); });`,
      );
    const countPosition = () =>
      evaluate(
        `const r = document.querySelector('.channel-section-toggle .section-count').getBoundingClientRect(); return [r.x, r.y];`,
      );
    // Names keep their line whatever happens to voice; touch hides a live row's shortcut.
    const names = () => geometry().map((row) => [row[0][0], row[0][1], row[0][3]]);
    // What stays put while your call's status takes a row's join slot: name, menu and line.
    const kept = (row) => [row[0][0], row[0][1], row[0][3], row.at(-2), row.at(-1)];
    const checkChannelTargets = () => {
      for (const area of ["leading padding", "icon", "name", "trailing padding"]) {
        for (const name of ["beta", "alpha"]) {
          const [x, y] =
            evaluate(`const button = [...document.querySelectorAll('.channel-select')].find(button => button.querySelector('span').textContent === ${JSON.stringify(name)});
            const r = button.getBoundingClientRect(), area = ${JSON.stringify(area)};
            const target = area === 'icon' ? button.querySelector('svg') : area === 'name' ? button.querySelector('span') : button;
            const t = target.getBoundingClientRect();
            const x = area === 'leading padding' ? r.left + 3 : area === 'trailing padding' ? r.right - 3 : t.left + Math.min(t.width / 2, 20);
            if (!button.contains(document.elementFromPoint(x, r.top + r.height / 2))) throw Error('Channel target is obscured: ' + area);
            if (getComputedStyle(target).cursor !== 'pointer') throw Error('Channel target needs a hand cursor: ' + area);
            return [Math.round(x), Math.round(r.top + r.height / 2)];`);
          browser("mouse", "move", String(x), String(y));
          browser("mouse", "down", "left");
          browser("mouse", "up", "left");
          wait(
            `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === ${JSON.stringify(name)} && document.querySelector('.chat-heading h2')?.textContent.includes(${JSON.stringify(name)})`,
          );
        }
      }
      for (const selector of [".space-rail button:enabled", ".call-account button:enabled", ".space-menu summary"]) {
        assert.ok(
          evaluate(
            `const buttons = [...document.querySelectorAll(${JSON.stringify(selector)})]; return buttons.length > 0 && buttons.every(button => getComputedStyle(button).cursor === 'pointer');`,
          ),
          `${selector}: enabled controls need hand cursors`,
        );
      }
      assert.ok(
        evaluate(
          `return [...document.querySelectorAll('.channel-manage')].every(button => getComputedStyle(button).cursor === 'pointer');`,
        ),
        "Owner channel options need hand cursors",
      );
      assert.ok(
        evaluate(`const button = document.querySelector('.call-settings-trigger');
        button.disabled = true; const disabled = getComputedStyle(button).cursor; button.disabled = false;
        button.setAttribute('aria-disabled', 'true'); const ariaDisabled = getComputedStyle(button).cursor; button.removeAttribute('aria-disabled');
        return disabled !== 'pointer' && ariaDisabled !== 'pointer';`),
        "Disabled controls must not advertise a click",
      );
    };
    for (const width of [1280, 390]) {
      browser("set", "viewport", String(width), "900", "2");
      evaluate(`await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`);
      if (width === 390) click("Back to Browse");
      browser("mouse", "move", "5", "5");
      evaluate(`document.activeElement.blur();`);
      const joinsBeforeSettings = evaluate(
        `return voiceFixture.commands.filter(c => c.method === 'media.join').length;`,
      );
      const before = geometry();
      assert.equal(before.length, 3, "Every channel, including empty/unselected/private channels, is measured");
      const fine = evaluate(`return matchMedia('(hover: hover) and (pointer: fine)').matches;`);
      assert.ok(
        evaluate(
          `return [...document.querySelectorAll('.channel-line')].every(line => !line.querySelector('.channel-live') && line.querySelectorAll('.channel-quick-join').length === 1);`,
        ),
        "Empty channels are one line, each with one join shortcut",
      );
      assert.ok(
        evaluate(
          `return [...document.querySelectorAll('.channel-quick-join')].every(button => button.getAttribute('aria-label') === 'Join voice in #' + button.closest('.channel-line').querySelector('.channel-select > span').textContent && !!button.querySelector('svg') && !button.textContent.trim());`,
        ),
        "Row shortcuts are icon-only and name their channel",
      );
      assert.ok(
        evaluate(
          `return [...document.querySelectorAll('.channel-line')].every(line => { const name = line.querySelector('.channel-select').getBoundingClientRect(), join = line.querySelector('.channel-quick-join'), style = getComputedStyle(join), r = join.getBoundingClientRect(); return Math.abs(r.top + r.height / 2 - (name.top + name.height / 2)) <= 1 && (${fine} ? r.width === 28 && style.opacity === '0' : r.width >= 44 && style.opacity === '1' && style.color === 'rgb(104, 113, 116)'); });`,
        ),
        "Fine pointers keep a hidden 28px shortcut on the name's line; touch shows a dim 44px one on quiet rows",
      );
      assert.ok(
        evaluate(
          `return [...document.querySelectorAll('.channel-line')].every(line => { const name = line.querySelector('.channel-select').getBoundingClientRect(); return matchMedia('(hover: hover) and (pointer: fine) and (min-width: 761px)').matches ? name.height === 32 : name.height >= 44; });`,
        ),
        "Compact desktop rows; mobile/no-hover keeps 44px targets",
      );
      assert.equal(
        evaluate(`return document.querySelectorAll('.channel-live, .live-card').length;`),
        0,
        "Empty channels show no zero-occupancy count and no live card",
      );
      assert.equal(
        evaluate(`return !!document.querySelector('.chat-heading :is(.channel-join, .voice-hangup, .header-voice)');`),
        false,
        "Joining lives in the sidebar, not the conversation header",
      );
      screenshot(`channel-${width}-empty`);
      if (fine) {
        browser("hover", '.channel-line:has([aria-label="Join voice in #beta"])');
        wait(`getComputedStyle(document.querySelector('[aria-label="Join voice in #beta"]')).opacity === '1'`);
        browser("hover", '[aria-label="Join voice in #beta"]');
        wait(
          `getComputedStyle(document.querySelector('[aria-label="Join voice in #beta"]')).color === 'rgb(243, 244, 245)'`,
        );
        assert.ok(
          evaluate(
            `const style = getComputedStyle(document.querySelector('[aria-label="Join voice in #beta"]')); return style.backgroundColor === 'rgb(52, 56, 59)' && style.color === 'rgb(243, 244, 245)';`,
          ),
          "Hover gives neutral feedback, not a primary-action fill",
        );
        assert.deepEqual(geometry(), before, "Revealing the shortcut cannot move the row");
        screenshot(`channel-${width}-hover`);
        browser("mouse", "move", "5", "5");
      }
      browser("focus", '.channel-select[aria-current="page"]');
      browser("press", "Tab");
      assert.equal(
        evaluate(`return document.activeElement.getAttribute('aria-label');`),
        "Join voice in #alpha",
        "Keyboard order runs name, join shortcut, then channel settings",
      );
      wait(
        `getComputedStyle(document.activeElement).backgroundColor === 'rgb(52, 56, 59)' && getComputedStyle(document.activeElement).color === 'rgb(243, 244, 245)'`,
      );
      assert.ok(
        evaluate(
          `const button = document.activeElement, style = getComputedStyle(button); return button.matches(':focus-visible') && style.outlineStyle === 'none' && style.opacity === '1';`,
        ),
        "Keyboard focus reveals the control with a neutral background and no ring",
      );
      assert.deepEqual(geometry(), before, "Keyboard feedback cannot move actions");
      screenshot(`channel-${width}-focus`);
      evaluate(`document.activeElement.blur();`);
      evaluate(
        `voiceFixture.publishPresence([{ id: 'peer', name: 'Mock peer', muted: false, deafened: false, tracks: [] }, { id: 'other', name: 'Other mock peer', muted: true, deafened: false, tracks: [] }]);`,
      );
      wait(`document.querySelector('.channel-live')?.getAttribute('aria-label') === '2 in voice'`);
      const occupied = geometry();
      // Every row is live here: each grows into its card, its name keeping its line.
      assert.deepEqual(
        names().map(([, top, height]) => [top, height]),
        before.map((row) => [row[0][1], row[0][3]]),
        "Live rows stay one line",
      );
      assert.ok(
        evaluate(`const mark = document.querySelector('.channel-live'), row = mark.closest('.channel-select').getBoundingClientRect(), r = mark.getBoundingClientRect();
          return mark.textContent === '2' && r.right <= row.right && r.top >= row.top && r.bottom <= row.bottom && getComputedStyle(mark).color === 'rgb(74, 168, 107)';`),
        "The live mark is a readable green count inside its row",
      );
      assert.equal(
        evaluate(
          `return getComputedStyle(document.querySelector('[aria-label="Join voice in #alpha"].channel-quick-join')).display === 'none';`,
        ),
        !fine,
        "Touch hides a live row's shortcut; its room card has Join",
      );
      browser("hover", '.channel-select[aria-current="page"]');
      browser("mouse", "move", "5", "5");
      assert.deepEqual(geometry(), occupied, "Hover cannot move any controls");
      click("Manage planning-for-the-next-release");
      assert.deepEqual(geometry(), occupied, "Opening channel actions must overlay, not reflow the channel list");
      assert.ok(
        evaluate(
          `const menu = document.querySelector('.channel-menu[open]'), actions = menu.querySelector('.space-actions').getBoundingClientRect(), row = menu.closest('.channel-line').getBoundingClientRect();
          return menu.dataset.placement === 'above' ? actions.bottom <= row.top : actions.top >= row.bottom;`,
        ),
        // A live card above can leave the last row too little room below.
        "Channel actions open below their own row, or above it when there is no room",
      );
      screenshot(`channel-${width}-menu`);
      browser("press", "Escape");
      assert.equal(
        evaluate(`return document.activeElement.getAttribute('aria-label');`),
        "Manage planning-for-the-next-release",
      );
      browser("press", "Enter");
      browser("mouse", "move", "5", "5");
      browser("mouse", "down", "left");
      browser("mouse", "up", "left");
      assert.equal(
        evaluate(`return !!document.querySelector('.channel-menu[open]');`),
        false,
        "Outside clicks dismiss channel actions",
      );
      click("Manage planning-for-the-next-release");
      // By selector: a role lookup here measured the item mid-entrance and pressed the one above.
      evaluate(
        `[...document.querySelectorAll('.channel-menu[open] .space-actions button')].find(button => button.textContent.trim() === 'Channel settings').dataset.testTarget = '';`,
      );
      browser("click", "[data-test-target]");
      wait(`document.querySelector('.space-dialog[open] .member-manager')`);
      assert.equal(evaluate(`return document.querySelector('.channel-privacy input').checked;`), true);
      browser("press", "Escape");
      assert.equal(
        evaluate(`return document.activeElement.getAttribute('aria-label');`),
        "Manage planning-for-the-next-release",
        "Settings dismissal must restore focus to the visible channel action",
      );
      assert.equal(
        evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').length;`),
        joinsBeforeSettings,
        "Opening settings must not connect voice",
      );
      screenshot(`channel-${width}-before-join`);
      // The live room's card joins in one press.
      browser("focus", '.live-room .live-join[aria-label="Join voice in #alpha"]');
      browser("press", "Enter");
      wait(
        `voiceFixture.client?.phase === 'connected' && document.querySelector('.live-call [aria-label="Leave voice"]')`,
      );
      browser("wait", "650");
      // The room's card becomes your call's: name, menu and line stay put (the status takes the shortcut's slot).
      assert.deepEqual(
        kept(geometry()[0]),
        kept(occupied[0]),
        "Connecting preserves the channel's name, menu and line",
      );
      assert.ok(
        evaluate(
          `return document.activeElement === document.querySelector('.live-call .voice-hangup') && !!document.querySelector('.live-call .lucide-phone-off') && document.querySelectorAll('[aria-label="Leave voice"]').length === 1;`,
        ),
        "Joining focuses Leave on your call card, the only one",
      );
      assert.equal(
        evaluate(`return document.querySelector('[data-channel="alpha0000000"].channel-quick-join');`),
        null,
        "Your call's channel offers no join shortcut",
      );
      assert.equal(
        evaluate(
          `return document.querySelector('.channel-quick-join[data-channel="beta00000000"]').getAttribute('aria-label');`,
        ),
        "Switch voice to #beta",
      );
      screenshot(`channel-${width}-joined`);
      if (width === 390) {
        browser("click", '.channel-select[aria-current="page"]');
        settleBrowse();
        assert.ok(
          evaluate(
            `const composer = document.querySelector('#chat-message'), r = composer.getBoundingClientRect(); return document.querySelector('.voice-strip .live-toggle').getBoundingClientRect().height >= 44 && document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === composer;`,
          ),
          "Narrow conversation keeps the strip's 44px Mute and unobscured text entry",
        );
        screenshot("channel-390-conversation");
        click("Back to Browse");
      }
      evaluate(
        `voiceFixture.joinedTrack = voiceFixture.captures.at(-1); voiceFixture.leaveCount = voiceFixture.commands.filter(c => c.method === 'media.leave').length;`,
      );
      chooseChannel("beta");
      wait(`document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'beta'`);
      assert.equal(
        evaluate(`return voiceFixture.joinedTrack.readyState;`),
        "live",
        "Selecting text must not leave the current voice channel",
      );
      assert.equal(
        evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`),
        evaluate(`return voiceFixture.leaveCount;`),
      );
      if (width === 390) openBrowse();
      // Touch hides a live row's shortcut; #beta's room card switches instead.
      browser("click", `${fine ? ".channel-line " : ".live-room .live-join"}[aria-label="Switch voice to #beta"]`);
      wait(`voiceFixture.client?.phase === 'connected' && ${connectedRow("beta")}`);
      assert.equal(
        evaluate(`return voiceFixture.joinedTrack.readyState;`),
        "ended",
        "Explicit switch must release the old capture",
      );
      assert.equal(
        evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).channelId;`),
        "beta00000000",
      );
      assert.equal(
        evaluate(`return document.querySelectorAll('[aria-label="Leave voice"]').length;`),
        1,
        "Switching keeps one call card",
      );
      click("Leave voice");
      wait(`voiceFixture.client.phase === 'idle'`);
      chooseChannel("alpha");
      wait(`document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha'`);
      if (width === 390) openBrowse();
      evaluate(
        `voiceFixture.publishPresence([]); voiceFixture.holdCapture = true; voiceFixture.releaseCapture = undefined;`,
      );
      wait(`!document.querySelector('.channel-live')`);
      const joins = evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').length;`);
      browser("click", '.channel-line [aria-label="Join voice in #alpha"]');
      // While the microphone is pending your call card says so and offers Cancel; no
      // second Join exists to press.
      wait(
        `document.querySelector('.live-call[data-phase="joining"] .live-status')?.textContent === 'Connecting…' && voiceFixture.releaseCapture`,
      );
      assert.equal(
        evaluate(`return document.querySelectorAll('[data-channel="alpha0000000"].channel-join').length;`),
        0,
      );
      assert.ok(
        evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').length;`) <= joins + 1,
        "One press issues at most one Join while capture is pending",
      );
      assert.deepEqual(
        geometry().map(kept),
        before.map(kept),
        "Pending microphone permission/capture must not shift the name, menu or line",
      );
      screenshot(`channel-${width}-joining`);
      click("Cancel joining voice");
      wait(`document.querySelector('[aria-label="Join voice in #alpha"]') && !document.querySelector('.live-call')`);
      evaluate(`voiceFixture.holdCapture = false; voiceFixture.releaseCapture();`);
      wait(`voiceFixture.captures.at(-1).readyState === 'ended'`);
      assert.deepEqual(geometry(), before, "Cancelled joins restore the same targets");
      evaluate(`voiceFixture.captureError = 'NotAllowedError';`);
      // Cancel's card is gone, so the pointer now rests on #beta's shortcut; its tooltip covers #alpha's.
      browser("mouse", "move", "5", "5");
      wait(`!document.querySelector('.top-layer-tooltip:popover-open')`);
      click("Join voice in #alpha");
      wait(`voiceFixture.client.phase === 'failed' && document.querySelector('.voice-error')`);
      assert.deepEqual(geometry(), before, "Failed joins keep the channel controls stable and retryable");
      assert.equal(
        evaluate(`return document.querySelector('[aria-label="Join voice in #alpha"]').getAttribute('aria-disabled');`),
        "false",
      );
      screenshot(`channel-${width}-error`);
      evaluate(`voiceFixture.captureError = undefined;`);
      click("Dismiss voice error");
      browser("set", "media", "reduced-motion");
      assert.equal(
        evaluate(`return getComputedStyle(document.querySelector('.channel-line')).transitionDuration;`),
        "0s",
      );
      browser("set", "media", "no-preference");
      assert.deepEqual(geometry(), before);
      screenshot(`channel-${width}-left`);
      if (width === 1280) {
        browser("focus", '[aria-label="Channel sidebar width"]');
        browser("press", "Home");
        assert.equal(evaluate(`return document.querySelector('.people-panel').getBoundingClientRect().width;`), 220);
        assert.ok(
          evaluate(
            `return [...document.querySelectorAll('.channel-line')].every(line => line.querySelector('.channel-quick-join').getBoundingClientRect().right <= line.getBoundingClientRect().right);`,
          ),
          "Shortcuts must fit at the 220px sidebar minimum",
        );
        assert.equal(
          evaluate(`return document.querySelectorAll('.channel-live').length;`),
          0,
          "Minimum-width empty rows stay one line",
        );
        checkChannelTargets();
        screenshot("channel-minimum-sidebar");
        evaluate(`document.activeElement.blur();`);
        const [x, y] = evaluate(
          `const r = document.querySelector('.channel-sidebar-resize').getBoundingClientRect(); return [Math.round(r.left + r.width / 2), Math.round(r.top + 100)];`,
        );
        browser("mouse", "move", String(x), String(y));
        browser("mouse", "down", "left");
        browser("mouse", "move", String(x + 67), String(y));
        assert.equal(
          evaluate(`return document.querySelector('.people-panel').getBoundingClientRect().width;`),
          287,
          "Pointer dragging must still resize the sidebar",
        );
        assert.equal(
          evaluate(`return getComputedStyle(document.querySelector('.channel-sidebar-resize')).cursor;`),
          "col-resize",
        );
        assert.equal(
          evaluate(`return getComputedStyle(document.querySelector('.channel-sidebar-resize'), '::after').content;`),
          "none",
          "Hover/drag must not create a colored resize line",
        );
        screenshot("channel-sidebar-drag-no-highlight");
        browser("mouse", "up", "left");
        browser("focus", '[aria-label="Channel sidebar width"]');
        browser("press", "Home");
        browser("press", "ArrowRight");
        browser("press", "ArrowRight");
        browser("press", "ArrowRight");
        browser("press", "ArrowRight");
      } else {
        assert.ok(
          before.every((row) => row[0][3] >= 44 && row[2][2] >= 44 && row[2][3] >= 44),
          "Narrow controls need 44px tap targets",
        );
      }
    }
    const ownerNarrowCount = countPosition();
    browser("set", "viewport", "1280", "900", "2");
    evaluate(`await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));`);
    const ownerDesktopCount = countPosition();
    browser("hover", ".channel-section-heading");
    browser("mouse", "move", "5", "5");
    assert.deepEqual(countPosition(), ownerDesktopCount, "Revealing owner section actions cannot shift the count");
    screenshot("channel-1280-owner");
    // (The fixture keeps your last call's roster; nobody is talking for this comparison.)
    evaluate(`voiceFixture.publishPresence([]); voiceFixture.showSpaces(false);`);
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha' && document.querySelector('.channel-manage') && !document.querySelector('.live-card')`,
    );
    browser("click", '[aria-label="Manage alpha"]');
    assert.equal(
      evaluate(`return document.querySelector('.channel-menu[open]').textContent.includes('Channel settings');`),
      false,
      "Members cannot access owner settings",
    );
    assert.equal(
      evaluate(`return document.querySelector('.channel-menu[open]').textContent.includes('Leave channel');`),
      true,
      "Members can leave from channel options",
    );
    browser("press", "Escape");
    assert.equal(
      evaluate(`return document.querySelectorAll('.channel-quick-join').length;`),
      3,
      "Members retain each row's join shortcut alongside channel options",
    );
    assert.deepEqual(
      countPosition(),
      ownerDesktopCount,
      "Owned and shared spaces keep the channel count in the same position",
    );
    checkChannelTargets();
    screenshot("channel-1280-member");
    browser("set", "viewport", "390", "900", "2");
    click("Back to Browse");
    assert.deepEqual(countPosition(), ownerNarrowCount, "Narrow owned and shared spaces also align the count");
    screenshot("channel-390-member");
    evaluate(`await voiceFixture.cleanup();`);
    console.log(
      "PASS one-line channel rows with hover/focus (touch: dim) join shortcuts at 1280px/390px and 220px sidebar, live marks, voice-free header, room-card join and card Leave/Cancel, neutral shortcut feedback and visible keyboard focus, owned/shared count alignment, empty/live/private channels, owner menu and focus return, join/leave/switch/cancel, browsing preserves voice, busy guard, reduced motion and member permissions (mock signaling/WebRTC)",
    );
  } else {
    for (const label of ["Input Options", "Output Options", "User Settings"]) {
      click(label);
      wait(
        `document.querySelector('.call-settings-panel').getAnimations().every(animation => animation.playState === 'finished')`,
      );
      for (const position of ["top", "side", "bottom"]) {
        const [x, y] = evaluate(
          `const r = document.querySelector('.call-settings-panel').getBoundingClientRect(); return ${JSON.stringify(position)} === 'top' ? [Math.round(r.left + r.width / 2), Math.round(r.top + 3)] : ${JSON.stringify(position)} === 'side' ? [Math.round(r.right - 3), Math.round(r.top + r.height / 2)] : [Math.round(r.left + r.width / 2), Math.round(r.bottom - 3)];`,
        );
        browser("mouse", "move", String(x), String(y));
        browser("mouse", "down", "left");
        browser("mouse", "up", "left");
        assert.equal(
          evaluate(`return document.querySelector('button[aria-label="${label}"]').getAttribute('aria-expanded');`),
          "true",
          "Empty-space clicks must leave the menu open",
        );
        assert.ok(
          evaluate(`return document.querySelector('.call-settings-panel').contains(document.activeElement);`),
          "Panel background safely takes focus",
        );
      }
      browser("press", "Escape");
      assert.equal(evaluate(`return document.activeElement.getAttribute('aria-label');`), label);
      assert.equal(evaluate(`return !!document.querySelector('.call-settings-panel');`), false);
      click(label);
      browser("click", ".chat-heading");
      assert.equal(
        evaluate(`return !!document.querySelector('.call-settings-panel');`),
        false,
        "Outside click still dismisses settings",
      );
    }
    console.log("PASS empty-panel clicks, Escape focus restoration, and outside dismissal");
    const bounds = () =>
      evaluate(
        `return ['.chat-heading', '.chat-composer'].map(selector => { const r = document.querySelector(selector).getBoundingClientRect(); return [r.top, r.height]; });`,
      );
    const initialBounds = bounds();
    assert.equal(
      evaluate(`return document.querySelector('.account-name').textContent;`),
      "UI fixture",
      "Profile must use display name, not username",
    );
    assert.equal(
      evaluate(`return document.querySelector('.call-page').textContent.includes('Talk here, or keep typing');`),
      false,
    );
    assert.ok(
      evaluate(
        // The 40px account row plus its panel/dock border pixels matches the 42px field (#451).
        `const panel = document.querySelector('.voice-panel'), dock = panel.querySelector('.voice-dock');
        const px = (element, side) => parseFloat(getComputedStyle(element)['border' + side + 'Width']);
        const a = document.querySelector('.call-account').getBoundingClientRect(), b = document.querySelector('#chat-message').getBoundingClientRect();
        return a.top - (dock ? px(dock, 'Bottom') : px(panel, 'Top')) === b.top && a.bottom + px(panel, 'Bottom') === b.bottom;`,
      ),
      "Account bar and single-line composer must align",
    );
    // Only touch layouts show a send button; Enter sends here.
    assert.equal(
      evaluate(
        `const send = document.querySelector('.chat-composer button[type="submit"]'); return !!send && getComputedStyle(send).display !== 'none';`,
      ),
      false,
    );
    browser("click", ".account-profile");
    wait(`document.querySelector('.profile-dialog[open] input')`);
    browser("fill", "#username", "taken");
    browser("click", '.profile-dialog button[type="submit"]');
    wait(`document.querySelector('.profile-dialog [role="alert"]')`);
    assert.equal(
      evaluate(`return document.querySelector('.profile-dialog [role="alert"]').textContent;`),
      "That username is already taken.",
    );
    browser("fill", "#username", "updated_fixture");
    browser("fill", "#display-name", "Updated fixture");
    browser("click", '.profile-dialog button[type="submit"]');
    wait(`!document.querySelector('dialog[open]')`);
    assert.equal(evaluate(`return document.querySelector('.account-name').textContent;`), "Updated fixture");
    assert.equal(evaluate(`return document.activeElement.className;`), "account-profile");
    browser("click", ".account-profile");
    wait(`document.querySelector('.profile-dialog[open] input')`);
    assert.equal(evaluate(`return document.querySelector('#username').value;`), "updated_fixture");
    browser("press", "Escape");
    wait(`!document.querySelector('dialog[open]')`);
    click("Input Options");
    assert.equal(
      evaluate(
        `return !!document.querySelector('.audio-device-summary') && !document.querySelector('[role="menuitemradio"]');`,
      ),
      true,
      "Only the selected device appears beside volume until its picker opens",
    );
    browser("focus", '[aria-label="Input volume"]');
    browser("press", "End");
    assert.equal(
      evaluate(`return document.querySelector('[aria-label="Input volume"]').getAttribute('aria-valuenow');`),
      "200",
    );
    browser("press", "Home");
    assert.equal(
      evaluate(`return document.querySelector('[aria-label="Input volume"]').getAttribute('aria-valuenow');`),
      "0",
    );
    browser("press", "End");
    wait(
      `getComputedStyle(document.querySelector('button[aria-label="Input Options"] svg')).transform === 'matrix(-1, 0, 0, -1, 0, 0)'`,
    );
    assert.equal(
      evaluate(`return getComputedStyle(document.querySelector('button[aria-label="Input Options"] svg')).transform;`),
      "matrix(-1, 0, 0, -1, 0, 0)",
      "Open chevron must point up",
    );
    click("Input device");
    screenshot("voice-input-dropdown");
    browser("press", "Escape");
    assert.ok(
      evaluate(
        `return !!document.querySelector('.call-settings-panel') && !document.querySelector('.audio-device-drawer') && document.activeElement.getAttribute('aria-label') === 'Input device';`,
      ),
      "Escape closes device choices before its parent panel and restores focus",
    );
    click("Input device");
    browser("press", "ArrowDown");
    browser("press", "Enter");
    assert.ok(
      evaluate(`return document.querySelector('.audio-device-summary').textContent.includes('Headset microphone');`),
    );
    screenshot("voice-input-options");
    click("Output Options");
    assert.equal(
      evaluate(`return document.querySelector('button[aria-label="Input Options"]').getAttribute('aria-expanded');`),
      "false",
    );
    click("Output device");
    assert.equal(
      evaluate(`return document.querySelectorAll('[role="menuitemradio"]').length;`),
      5,
      "System default and its physical device remain distinct choices",
    );
    browser("find", "role", "menuitemradio", "click", "--name", "Headphones", "--exact");
    assert.ok(evaluate(`return document.querySelector('.audio-device-summary').textContent.includes('Headphones');`));
    click("Output device");
    assert.equal(evaluate(`return document.querySelector('[aria-checked="true"]').textContent;`), "Headphones");
    screenshot("voice-output-choices");
    browser("press", "Escape");
    browser("focus", '[aria-label="Output volume"]');
    browser("press", "PageDown");
    assert.equal(
      evaluate(`return document.querySelector('.call-settings-panel .output-volume output').textContent;`),
      "90%",
    );
    screenshot("voice-output-options");
    browser("press", "Escape");
    assert.equal(
      evaluate(`return document.querySelector('button[aria-label="Output Options"]').getAttribute('aria-expanded');`),
      "false",
    );
    click("User Settings");
    assert.equal(evaluate(`return !!document.querySelector('.call-settings-panel [role="slider"]');`), false);
    assert.equal(
      evaluate(`return document.querySelector('.call-settings-panel').textContent.includes('@fixture');`),
      false,
    );
    assert.equal(evaluate(`return !!document.querySelector('.call-settings-panel a[href="/profile"]');`), false);
    assert.ok(
      evaluate(
        `return [...document.querySelectorAll('.call-settings-panel button')].every(button => { const style = getComputedStyle(button); return style.borderTopWidth === '0px' && style.backgroundColor === 'rgba(0, 0, 0, 0)'; });`,
      ),
      "Menu actions must not look like bordered inputs",
    );
    screenshot("voice-settings");
    browser("press", "Escape");

    click("Mute microphone");
    assert.equal(
      evaluate(`return document.querySelector('[aria-label="Unmute microphone"]').getAttribute('aria-pressed');`),
      "true",
      "Mute must work before joining",
    );
    click("Deafen audio");
    assert.equal(
      evaluate(`return document.querySelector('[aria-label="Undeafen audio"]').getAttribute('aria-pressed');`),
      "true",
      "Deafen must work before joining",
    );
    assert.ok(
      evaluate(`return matchMedia('(hover: hover) and (pointer: fine)').matches;`),
      'Hover states need VOICE_TEST_CHROME="$PWD/scripts/fine-pointer-chromium.sh"',
    );
    browser("hover", '[aria-label="Undeafen audio"]');
    wait(`getComputedStyle(document.querySelector('[aria-label="Undeafen audio"]')).color === 'rgb(255, 113, 130)'`);
    assert.equal(
      evaluate(`return getComputedStyle(document.querySelector('[aria-label="Output Options"]')).color;`),
      "rgb(237, 82, 101)",
      "Dropdown must share the active red state",
    );
    screenshot("voice-prejoin-deafened-hover");
    click("Undeafen audio");
    if (evaluate(`return !!document.querySelector('[aria-label="Unmute microphone"]');`)) click("Unmute microphone");
    // One click on the channel row joins; your call card appears at the top of the channels.
    click("Join voice in #general");
    wait(`document.querySelector('.live-call [aria-label="Leave voice"]')`);
    assert.equal(
      evaluate(`return !!document.querySelector('.chat-header .voice-hangup, .chat-header .channel-join');`),
      false,
      "Voice stays out of the conversation header",
    );
    assert.equal(evaluate(`return voiceFixture.sockets.length;`), 1, "Chat and joined voice must share one socket");
    assert.deepEqual(bounds(), initialBounds, "Joining must not move the chat header or composer");
    assert.equal(evaluate(`return voiceFixture.devices.at(-1);`), "headset");
    evaluate(`voiceFixture.addRemoteAudio();`);
    wait(`voiceFixture.outputGain`);
    const gain = () => evaluate(`return voiceFixture.outputGain.gain.value;`);
    assert.ok(Math.abs(gain() - 0.9) < 0.00001, `Master volume must reach the audio graph (gain ${gain()})`);
    // A right-click on your call card's faces opens the full list, where per-person audio lives.
    const openEveryone = () => {
      if (!evaluate(`return !!document.querySelector('.live-everyone:popover-open');`))
        rightClick(".live-call .live-face");
      wait(`document.querySelector('.live-everyone:popover-open')`);
    };
    openEveryone();
    if (evaluate(`return matchMedia('(hover: hover) and (pointer: fine)').matches;`)) {
      browser("click", ".live-everyone-heading p");
      assert.equal(
        evaluate(`return getComputedStyle(document.querySelector('.participant-menu-button')).opacity;`),
        "0",
      );
      browser("focus", ".participant-menu-button");
      wait(
        `document.querySelector('.participant-menu-button').matches(':focus') && getComputedStyle(document.querySelector('.participant-menu-button')).opacity === '1'`,
      );
      assert.equal(
        evaluate(`return getComputedStyle(document.querySelector('.participant-menu-button')).opacity;`),
        "1",
        "Keyboard focus reveals Audio",
      );
      browser("click", ".live-everyone-heading p");
    }
    browser("hover", ".participant:has(.participant-menu-button)");
    // Hover reveals Audio through a short opacity transition.
    wait(`getComputedStyle(document.querySelector('.participant-menu-button')).opacity === '1'`);
    click("Audio controls for Peach Donkey");
    browser("focus", '[aria-label="Peach Donkey volume"]');
    browser("press", "PageUp");
    wait(
      `document.querySelector('[aria-label="Peach Donkey volume"]').getAttribute('aria-valuenow') === '110' && Math.abs(voiceFixture.outputGain.gain.value - 0.99) < 0.00001`,
    );
    assert.ok(Math.abs(gain() - 0.99) < 0.00001, "90% master × 110% participant must produce 99% gain");
    screenshot("participant-audio");
    browser("click", ".chat-heading");
    assert.equal(
      evaluate(`return !!document.querySelector('.participant-volume');`),
      false,
      "Outside click closes participant controls",
    );
    assert.equal(
      evaluate(`return !!document.querySelector('.live-everyone');`),
      false,
      "Outside click closes the full list too",
    );
    // At every sidebar width the full list opens beside the sidebar, inside the viewport.
    const everyoneBeside = () => {
      openEveryone();
      assert.ok(
        evaluate(
          `const list = document.querySelector('.live-everyone').getBoundingClientRect(), panel = document.querySelector('.people-panel').getBoundingClientRect(); return list.left >= panel.right - 1 && list.right <= innerWidth && list.top >= 0 && list.bottom <= innerHeight;`,
        ),
        "Full list opens beside the sidebar",
      );
      browser("press", "Escape");
      wait(`!document.querySelector('.live-everyone')`);
    };
    for (const key of ["Home", "End", "Home", "End"]) {
      browser("focus", ".channel-sidebar-resize");
      browser("press", key);
      everyoneBeside();
    }
    for (const width of [250, 330, 440]) {
      const [x, y, target] = evaluate(
        `const r = document.querySelector('.channel-sidebar-resize').getBoundingClientRect(); const p = document.querySelector('.people-panel').getBoundingClientRect(); return [r.left + r.width / 2, r.top + 100, p.left + ${width}];`,
      );
      browser("mouse", "move", String(x), String(y));
      browser("mouse", "down", "left");
      browser("mouse", "move", String(target), String(y));
      browser("mouse", "up", "left");
      everyoneBeside();
    }
    click("Input Options");
    assert.ok(
      evaluate(`return document.querySelector('.call-settings-panel').getBoundingClientRect().width <= 280;`),
      "Input menu stays compact in a wide sidebar",
    );
    screenshot("compact-input-wide-sidebar");
    browser("press", "Escape");
    openEveryone();
    browser("hover", ".participant:has(.participant-menu-button)");
    click("Audio controls for Peach Donkey");
    browser("press", "Escape");
    assert.equal(evaluate(`return !!document.querySelector('.participant-volume');`), false);
    assert.ok(
      evaluate(`return !!document.querySelector('.live-everyone:popover-open');`),
      "Escape closes the volume menu before the list",
    );
    click("Audio controls for Peach Donkey");
    click("Audio controls for Peach Donkey");
    browser("press", "Escape");
    wait(`!document.querySelector('.live-everyone')`);
    click("Output Options");
    browser("focus", '[aria-label="Output volume"]');
    browser("press", "Home");
    wait(`voiceFixture.outputGain.gain.value === 0`);
    assert.equal(gain(), 0, "Zero master volume must silence playback");
    browser("press", "End");
    wait(`Math.abs(voiceFixture.outputGain.gain.value - 2.2) < 0.00001`);
    assert.ok(Math.abs(gain() - 2.2) < 0.00001, "200% master × 110% participant must produce 220% gain");
    browser("press", "PageDown");
    browser("press", "Escape");
    click("Mute microphone");
    wait(`document.querySelector('[aria-label="Unmute microphone"]')`);
    click("Deafen audio");
    wait(`document.querySelector('[aria-label="Undeafen audio"]')`);
    evaluate(`clearInterval(voiceFixture.client.statsTimer);`);
    evaluate(
      `voiceFixture.client.diagnostics = { join: 'Fixture join', microphoneSessionMs: 10, signalingMs: 20, transportMs: 30, rosterMs: 40, receivedBytes: 10000, receiveBitrate: 32000, sentBytes: 10000, sendBitrate: 32000, packetsLost: 0, maxJitterMs: 1, roundTripMs: 12, route: 'direct' }; voiceFixture.client.emit();`,
    );
    click("User Settings");
    browser("find", "role", "button", "click", "--name", "Connection details", "--exact");
    wait(`document.querySelector('dialog[open] .call-diagnostics')`);
    assert.deepEqual(bounds(), initialBounds, "Diagnostics must overlay rather than reflow messages");
    centeredDialog();
    evaluate(`navigator.clipboard.writeText = async text => { voiceFixture.copied = text; };`);
    browser("find", "role", "button", "click", "--name", "Copy connection details", "--exact");
    assert.equal(evaluate(`return JSON.parse(voiceFixture.copied).signalingMs;`), 20);
    assert.ok(
      evaluate(`return document.querySelector('dialog[open]').textContent.includes('Copied connection details');`),
    );
    screenshot("voice-connection-details");
    browser("mouse", "move", "5", "5");
    browser("mouse", "down", "left");
    browser("mouse", "up", "left");
    wait(`!document.querySelector('dialog[open]')`);
    assert.equal(
      evaluate(`return document.activeElement.getAttribute('aria-label');`),
      "User Settings",
      "Dialog close should restore focus to its settings trigger",
    );

    click("User Settings");
    browser("find", "role", "button", "click", "--name", "Audio diagnostics", "--exact");
    wait(`document.querySelector('dialog[open] .audio-debug')`);
    browser("click", ".audio-debug p:first-child");
    assert.ok(
      evaluate(`return !!document.querySelector('dialog[open]');`),
      "Inside click must not dismiss diagnostics",
    );
    browser("mouse", "move", "5", "5");
    browser("mouse", "down", "left");
    browser("mouse", "up", "left");
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
    evaluate(
      `voiceFixture.client.participants = [...voiceFixture.client.participants]; voiceFixture.client.emit(); await new Promise(r => setTimeout(r, 200));`,
    );
    assert.equal(
      evaluate(`return voiceFixture.effectStarts;`),
      1,
      "A departure plays once; repeated snapshots do not repeat it",
    );
    evaluate(
      `voiceFixture.client.participants = [...voiceFixture.people]; voiceFixture.client.emit(); await new Promise(r => setTimeout(r, 200));`,
    );
    // Arrivals chime with the join sound (#179), never the departure sound.
    wait(`voiceFixture.effectStarts >= 2`);
    assert.equal(evaluate(`return voiceFixture.effectStarts;`), 2, "An arrival plays one sound");
    assert.notEqual(
      evaluate(`return voiceFixture.effectDurations[1];`),
      evaluate(`return voiceFixture.effectDurations[0];`),
      "An arrival must not play the departure sound",
    );
    console.log(
      "PASS participant audio visibility/dismissal, full list beside resized sidebar, compact menu width, diagnostics copy/dismissal, and departure sound (mock signaling/WebRTC)",
    );

    click("User Settings");
    browser("find", "role", "button", "click", "--name", "Audio test", "--exact");
    wait(`document.querySelector('dialog[open] .mic-test-button')`);
    assert.deepEqual(bounds(), initialBounds, "Mic test must not reflow messages");
    browser("click", ".mic-test-button");
    wait(`document.querySelector('.recording-clock')`);
    centeredDialog();
    screenshot("voice-mic-test");
    click("Close audio test");
    wait(`!document.querySelector('dialog[open]')`);
    assert.equal(
      evaluate(`return voiceFixture.recorders.at(-1).state;`),
      "inactive",
      "Closing during recording must cancel it",
    );
    assert.equal(evaluate(`return voiceFixture.client.monitoring;`), false);
    assert.equal(
      evaluate(`return voiceFixture.client.muted && voiceFixture.client.deafened;`),
      true,
      "Closing restores the previous mute/deafen intent",
    );
    assert.equal(
      evaluate(`return voiceFixture.captures.at(-1).readyState;`),
      "live",
      "Connected mic test borrows the track; closing must not end the call",
    );
    assert.deepEqual(bounds(), initialBounds);

    evaluate(`voiceFixture.typing();`);
    wait(`document.querySelectorAll('.chat-typing-dots i').length === 3`);
    const motion = evaluate(
      `const dots = [...document.querySelectorAll('.chat-typing-dots i')]; const sample = () => dots.map(dot => getComputedStyle(dot).transform); const before = sample(); await new Promise(r => setTimeout(r, 250)); return { before, after: sample(), delays: dots.map(dot => getComputedStyle(dot).animationDelay) };`,
    );
    assert.notDeepEqual(motion.before, motion.after, "Typing dots must actually move");
    assert.equal(new Set(motion.delays).size, 3, "Typing dots must be staggered");
    screenshot("voice-controls-desktop");
    if (artifacts) {
      browser("record", "start", `${artifacts}/typing-dots.webm`);
      evaluate(`voiceFixture.typing(); await new Promise(r => setTimeout(r, 1800));`);
      browser("record", "stop");
    }
    browser("set", "media", "reduced-motion");
    assert.equal(
      evaluate(`return getComputedStyle(document.querySelector('.chat-typing-dots i')).animationName;`),
      "none",
    );
    browser("set", "media", "no-preference");

    browser("set", "viewport", "390", "844", "2");
    click("Input Options");
    assert.ok(
      evaluate(
        `const r = document.querySelector('.call-settings-panel').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth;`,
      ),
    );
    screenshot("voice-controls-narrow");
    browser("press", "Escape");
    click("Output Options");
    assert.ok(
      evaluate(
        `const r = document.querySelector('.call-settings-panel').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth;`,
      ),
    );
    screenshot("voice-output-narrow");
    click("Output device");
    assert.ok(
      evaluate(`const r = document.querySelector('.audio-device-drawer').getBoundingClientRect();
      return r.left >= 0 && r.right <= innerWidth && r.top >= 0 && r.bottom <= innerHeight;`),
    );
    screenshot("voice-output-choices-narrow");
    browser("press", "Escape");
    browser("press", "Escape");
    click("User Settings");
    screenshot("voice-settings-narrow");
    browser("press", "Escape");
    click("User Settings");
    browser("find", "role", "button", "click", "--name", "Audio test", "--exact");
    wait(`document.querySelector('dialog[open] .mic-test-button')`);
    centeredDialog();
    screenshot("voice-mic-test-narrow");
    // The narrow dialog scrolls; reveal the button inside it before clicking, or
    // the click lands on the backdrop beneath its clipped position.
    browser("scrollintoview", ".mic-test-button");
    browser("click", ".mic-test-button");
    wait(`document.querySelector('.recording-clock')`);
    evaluate(`await new Promise(resolve => setTimeout(resolve, 1000));`);
    browser("click", ".mic-test-button");
    wait(`document.querySelector('audio[aria-label="Natural audio sample"]')`);
    wait(`voiceFixture.sampleGains.length > 0`);
    assert.ok(
      evaluate(`return voiceFixture.sampleGains.every(node => Math.abs(node.gain.value - 1.9) < 0.00001);`),
      "Mic-test playback must amplify to 190% without setting native volume above one",
    );
    browser("press", "Escape");
    wait(`!document.querySelector('dialog[open]')`);
    wait(`voiceFixture.sampleGains.every(node => node.context.state === 'closed')`);
    const beforeDisconnect = evaluate(`return voiceFixture.effectStarts;`);
    click("Leave voice");
    wait(`document.querySelector('[aria-label="Join voice in #general"]') && !document.querySelector('.live-call')`);
    wait(`voiceFixture.effectStarts === ${beforeDisconnect + 1}`);
    assert.ok(
      evaluate(`return Math.abs(voiceFixture.effectDurations.at(-1) - 0.890159) < 0.001;`),
      "Self-disconnect plays the supplied 0.89-second WAV",
    );
    assert.notEqual(
      evaluate(`return voiceFixture.effectDurations[0];`),
      evaluate(`return voiceFixture.effectDurations.at(-1);`),
      "Remote departure and self-disconnect must be distinct sounds",
    );
    console.log("PASS supplied self-disconnect sound is distinct from remote departure");
    click("User Settings");
    browser("find", "role", "button", "click", "--name", "Audio test", "--exact");
    wait(`document.querySelector('dialog[open] .mic-test-button')`);
    click("Close audio test");
    wait(`!document.querySelector('dialog[open]')`);
    assert.equal(
      evaluate(`return voiceFixture.captures.at(-1).readyState;`),
      "ended",
      "Closing a pre-join test must release its microphone",
    );
    evaluate(`voiceFixture.holdCapture = true;`);
    click("User Settings");
    browser("find", "role", "button", "click", "--name", "Audio test", "--exact");
    wait(`document.querySelector('dialog[open] [role="status"]')`);
    click("Close audio test");
    evaluate(`voiceFixture.holdCapture = false; voiceFixture.captureError = 'NotFoundError';`);
    click("User Settings");
    browser("find", "role", "button", "click", "--name", "Audio test", "--exact");
    wait(`document.querySelector('dialog[open] [role="alert"]')`);
    assert.ok(evaluate(`return document.querySelector('dialog[open]').textContent.includes('Connect a microphone');`));
    assert.equal(
      evaluate(`return !!document.querySelector('dialog[open] > [role="status"]');`),
      false,
      "Mic preparation status must clear after capture fails",
    );
    evaluate(`voiceFixture.captureError = undefined;`);
    browser("find", "role", "button", "click", "--name", "Try again", "--exact");
    wait(`document.querySelector('dialog[open] .mic-test-button')`);
    evaluate(`voiceFixture.releaseCapture();`);
    wait(`voiceFixture.captures.at(-1).readyState === 'ended'`);
    assert.equal(
      evaluate(`return !!document.querySelector('dialog[open] .mic-test-button');`),
      true,
      "Late cancelled capture must not disturb the successful retry",
    );
    click("Close audio test");
    assert.equal(
      evaluate(
        `return !!document.querySelector('.call-controls') || /huddle/i.test(document.querySelector('.call-page').textContent);`,
      ),
      false,
    );
    browser("set", "viewport", "1280", "900", "2");
    evaluate(`voiceFixture.showSpaces();`);
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'alpha' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    click("Mute microphone");
    click("Join voice in #alpha");
    wait(`document.querySelector('.live-call [aria-label="Leave voice"]')`);
    assert.equal(
      evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).body.muted;`),
      true,
      "Join must transmit pre-join mute intent",
    );
    evaluate(
      `voiceFixture.joinedClient = voiceFixture.client; voiceFixture.callTrack = voiceFixture.captures.at(-1); voiceFixture.leaveCount = voiceFixture.commands.filter(c => c.method === 'media.leave').length; voiceFixture.settingsNode = document.querySelector('[aria-label="User Settings"]');`,
    );
    chooseChannel("beta");
    wait(
      `document.querySelector('.channel-select[aria-current="page"] > span')?.textContent === 'beta' && document.querySelector('.channel-join[aria-disabled="false"]')`,
    );
    assert.ok(
      evaluate(
        `return voiceFixture.client === voiceFixture.joinedClient && voiceFixture.callTrack.readyState === 'live';`,
      ),
      "Browsing beta must keep alpha voice alive",
    );
    assert.equal(
      evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`),
      evaluate(`return voiceFixture.leaveCount;`),
      "Navigation must not send Leave",
    );
    assert.ok(
      // Your call card stays at the top of the channels while you read another one.
      evaluate(`return ${connectedRow("alpha")} && !!document.querySelector('.live-call [aria-label="Leave voice"]');`),
      "Connected channel must remain visible",
    );
    assert.ok(
      evaluate(`return document.querySelector('[aria-label="User Settings"]') === voiceFixture.settingsNode;`),
      "Settings must not remount on navigation",
    );
    assert.ok(
      evaluate(
        `return !!document.querySelector('[aria-label="Switch voice to #beta"]') && !!document.querySelector('[aria-label="Unmute microphone"]');`,
      ),
    );
    screenshot("voice-browsing-another-channel");
    click("Switch voice to #beta");
    wait(`${connectedRow("beta")}`);
    assert.equal(
      evaluate(`return voiceFixture.callTrack.readyState;`),
      "ended",
      "Explicit Join must release the previous call",
    );
    assert.equal(
      evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').length;`),
      evaluate(`return voiceFixture.leaveCount + 1;`),
    );
    assert.equal(
      evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.leave').at(-1).channelId;`),
      "alpha0000000",
    );
    assert.equal(
      evaluate(`return voiceFixture.commands.filter(c => c.method === 'media.join').at(-1).channelId;`),
      "beta00000000",
    );
    assert.ok(
      evaluate(`return ${connectedRow("beta")} && voiceFixture.client.muted;`),
      "Voice switch must preserve mute",
    );
    click("Leave voice");
    wait(`!document.querySelector('.live-call, .channel-live[data-active]')`);
    evaluate(`voiceFixture.captureError = 'NotReadableError';`);
    click("Join voice in #beta");
    // Capture failures show in the voice panel beside the controls.
    wait(`document.querySelector('.voice-error')`);
    assert.ok(
      evaluate(`return document.querySelector('.voice-error').textContent.includes('another app');`),
      "Busy microphone errors must be actionable",
    );
    assert.ok(
      evaluate(
        `return document.querySelector('.chat-heading').getBoundingClientRect().top === document.querySelector('.channel-navigation > header').getBoundingClientRect().top;`,
      ),
      "Voice errors must not shift the channel header",
    );
    screenshot("voice-device-busy-error");
    evaluate(
      `voiceFixture.captureError = undefined; voiceFixture.holdCapture = true; voiceFixture.releaseCapture = undefined;`,
    );
    click("Join voice in #beta");
    wait(`document.querySelector('[aria-label="Cancel joining voice"]') && voiceFixture.releaseCapture`);
    click("Cancel joining voice");
    wait(`document.querySelector('[aria-label="Join voice in #beta"]') && !document.querySelector('.live-call')`);
    evaluate(`voiceFixture.holdCapture = false; voiceFixture.releaseCapture();`);
    wait(`voiceFixture.captures.at(-1).readyState === 'ended'`);
    assert.equal(
      evaluate(`return voiceFixture.client.phase;`),
      "idle",
      "Late microphone capture must not resurrect a cancelled join",
    );
    evaluate(`await voiceFixture.cleanup();`);
    console.log(
      "PASS voice navigation persistence, explicit channel switching, pre-join mute/deafen and red hover/dropdown states, busy-device errors, join cancellation, profile, audio settings, recording cleanup, and narrow layout (mock signaling/WebRTC)",
    );
  }
} catch (error) {
  screenshot("failure");
  console.error(browser("errors"));
  console.error(
    evaluate(
      `return { alerts: [...document.querySelectorAll('[role="alert"]')].map(node => node.textContent), phase: window.voiceFixture?.client?.phase, channels: [...document.querySelectorAll('.channel-select')].map(node => [node.textContent, node.getAttribute('aria-current')]), voice: [...document.querySelectorAll('.channel-join, .voice-hangup')].map(node => [node.getAttribute('aria-label'), node.getAttribute('aria-disabled')]), pages: document.querySelectorAll('.call-page').length };`,
    ),
  );
  throw error;
} finally {
  browser("close");
}
