// Real Call component and voice client on the shared application gateway, with a
// mocked WebSocket gateway, HTTP and WebRTC. No provider resources or real microphone.
// Run against Vite: node scripts/test-public-presence.mjs http://localhost:5174 [--screenshots]
// Gateway handoff, draining and command retry are unit-tested in apps/web/src/server/gateway.test.ts.
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";

const origin = new URL(process.argv[2] ?? "http://localhost:5174");
if (!["localhost", "127.0.0.1"].includes(origin.hostname)) throw new Error("Use local Vite, not production.");
const session = `presence-${process.pid}`;
const artifacts = process.argv.includes("--screenshots");
function browser(...args) {
  return execFileSync(
    "agent-browser",
    ["--session", session, "--args", "--autoplay-policy=no-user-gesture-required", ...args],
    { encoding: "utf8", timeout: 35_000 },
  );
}
function evaluate(code) {
  return browser("eval", `(async()=>{${code}})()`);
}

async function fixture() {
  const { default: React } = await import("/node_modules/.vite/deps/react.js");
  const {
    default: { createRoot },
  } = await import("/node_modules/.vite/deps/react-dom_client.js");
  // Use Call's exact client module URL, including Vite's HMR cache key after edits.
  const source = await (await fetch("/src/pages/Call.tsx")).text();
  const { PublicCallClient } = await import(source.match(/from "(\/src\/media\/client\.ts[^"]*)"/)[1]);
  const { default: Call } = await import("/src/pages/Call.tsx");
  document.querySelector("main").hidden = true;
  const label = document.createElement("p");
  label.textContent = "Regression fixture: mocked gateway/WebRTC, no live participants";
  document.body.append(label);
  const mount = (window.mount = document.createElement("div"));
  document.body.append(mount);
  window.control = (name) => mount.querySelector(`[aria-label="${name}"]`);
  // Your call card's faces, labelled like "Grok (you)", "Phone, muted" or "Phone, deafened".
  window.faces = () =>
    [...mount.querySelectorAll(".live-call .live-face")].map((face) => face.getAttribute("aria-label"));
  window.roster = () => faces().map((label) => label.split(", ")[0].replace(/ \(you\)$/, ""));
  window.row = (name) => faces().find((label) => label.split(", ")[0].replace(/ \(you\)$/, "") === name);
  // Everyone in voice in #general, in a call or not: its row's "N in voice" mark. This
  // single-channel Call's only row is #general, whose mark stays on its line as a room card.
  window.inVoice = () =>
    Number(/^(\d+) in voice$/.exec(mount.querySelector(".channel-live")?.getAttribute("aria-label") ?? "")?.[1] ?? 0);
  const f = (window.fixture = {
    people: [],
    revision: 0,
    sockets: [],
    held: [],
    holdSockets: false,
    mediaFetches: 0,
    subscribes: 0,
    closes: 0,
    failClose: false,
    leavePending: false,
    peers: [],
  });
  window.connected = () => !!control("Leave voice") && f.client?.phase === "connected";
  const account = { id: "account", username: "fixture", displayName: "Grok", avatarId: 0 };
  const author = { id: account.id, name: account.displayName, isGuest: false };
  const snapshot = (spectator) => ({
    type: "snapshot",
    participants: f.people.map((person) => (spectator ? (({ tracks, ...rest }) => rest)(person) : person)),
    revision: f.revision,
  });
  // A roster change reaches every media subscription; spectators never see tracks.
  window.pushRoster = (people) => {
    f.people = people;
    f.revision++;
    for (const socket of f.sockets)
      for (const [id, request] of socket.subscriptions)
        if (request.kind === "media") socket.frame({ type: "event", id, event: snapshot(!request.token) });
  };
  const command = async (method, body) => {
    if (method === "media.join") {
      f.joinIntent = body;
      pushRoster([
        {
          id: "self",
          name: account.displayName,
          muted: body.muted ?? false,
          deafened: body.deafened ?? false,
          tracks: [],
        },
      ]);
      return [200, { token: "fixture-capability", id: "self", iceServers: [] }];
    }
    if (method === "media.publish") return [200, { sessionDescription: { type: "answer", sdp: "v=0" } }];
    if (method === "media.subscribe") {
      f.subscribes++;
      return [200, { tracks: [{ mid: `remote-${f.subscribes}` }], requiresImmediateRenegotiation: false }];
    }
    if (method === "media.close") {
      f.closes++;
      return f.failClose ? [503, { error: "cleanup unavailable" }] : [204, null];
    }
    if (method === "media.state") {
      const self = f.people.find((person) => person.id === "self");
      if (self && (self.muted !== body.muted || self.deafened !== body.deafened))
        pushRoster(f.people.map((person) => (person.id === "self" ? { ...person, ...body } : person)));
      return [204, null];
    }
    if (method === "media.leave") {
      f.leavePending = true;
      await new Promise((resolve) => {
        f.commitLeave = () => {
          pushRoster([]);
          f.leavePending = false;
          resolve();
        };
      });
      return [204, null];
    }
    if (method === "media.snapshot") return [200, { participants: f.people, revision: f.revision }];
    if (method === "media.warm") return [422, { error: "Warm voice is not supported by this fixture" }];
    if (method === "media.prepare" || method === "typing") return [204, null];
    return [404, { error: `Unexpected fixture command: ${method}` }];
  };
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === "string" ? input : input.url, location.href).pathname;
    if (path === "/api/media/status") return Response.json({ enabled: true });
    // Media control and rosters must use the gateway, never HTTP polling.
    if (path.startsWith("/api/media/")) f.mediaFetches++;
    if (path === "/api/account/me") return Response.json(account);
    if (path === "/api/chat/session") return Response.json({ token: "fixture-only", author });
    if (path === "/api/chat/general")
      return Response.json({
        space: { id: "fixture", name: "Presence fixture" },
        channel: { id: "general", name: "general" },
        cursor: "0",
        hasMore: false,
        messages: [],
      });
    if (path.startsWith("/api/")) return Response.json({ error: `Disabled in fixture: ${path}` }, { status: 404 });
    return originalFetch(input, options);
  };
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    subscriptions = new Map();
    constructor(url, protocols) {
      super();
      if (!String(url).includes("/api/chat/events")) return new NativeSocket(url, protocols);
      f.sockets.push(this);
      const hello = () => this.frame({ type: "hello", idleTimeoutSeconds: 600, serverTime: Date.now() });
      if (f.holdSockets) f.held.push(hello);
      else setTimeout(hello, 0);
    }
    async send(data) {
      const request = JSON.parse(data);
      if (request.type === "heartbeat") this.frame({ type: "heartbeat" });
      if (request.type === "unsubscribe") this.subscriptions.delete(request.id);
      if (request.type === "subscribe") {
        this.subscriptions.set(request.id, request);
        const event =
          request.kind === "chat" || request.kind === "notifications"
            ? { type: "ready", cursor: request.after ?? "0" }
            : request.kind === "presence"
              ? {
                  type: "snapshot",
                  members: (request.allMembers ? [account.id] : request.userIds).map((userId) => ({
                    userId,
                    status: "online",
                  })),
                }
              : snapshot(!request.token);
        this.frame({ type: "event", id: request.id, event });
        this.frame({ type: "subscribed", id: request.id });
      }
      if (request.type === "command") {
        const [status, body] = await command(request.method, request.body ?? {});
        this.frame({ type: "result", id: request.id, status, body });
      }
    }
    frame(value) {
      this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(value) }));
    }
    close() {
      f.sockets = f.sockets.filter((socket) => socket !== this);
    }
  };
  // Drop every gateway socket, as a network loss or a gateway restart would.
  window.dropSockets = () => {
    for (const socket of f.sockets) {
      socket.close();
      socket.dispatchEvent(new Event("close"));
    }
  };
  window.releaseSockets = () => {
    f.holdSockets = false;
    for (const hello of f.held.splice(0)) hello();
  };
  const context = new AudioContext();
  await context.resume();
  navigator.mediaDevices.enumerateDevices = async () => [
    { deviceId: "default", groupId: "fixture", kind: "audioinput", label: "Mock microphone" },
    { deviceId: "default", groupId: "fixture", kind: "audiooutput", label: "Mock speakers" },
  ];
  PublicCallClient.prototype.prepareMicrophone = () => {};
  PublicCallClient.prototype.openMicrophone = async function () {
    f.client = this;
    return context.createMediaStreamDestination().stream.getAudioTracks()[0];
  };
  window.RTCPeerConnection = class extends EventTarget {
    connectionState = "connected";
    iceConnectionState = "connected";
    iceGatheringState = "complete";
    senders = [];
    configuration = {};
    constructor() {
      super();
      f.peers.push(this);
    }
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
    async createAnswer() {
      return { type: "answer", sdp: "v=0" };
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
    getTransceivers() {
      return [];
    }
    async getStats() {
      return new Map();
    }
    close() {
      this.connectionState = "closed";
    }
  };
  const root = createRoot(mount);
  root.render(React.createElement(Call));
  window.cleanup = async () => {
    root.unmount();
    await context.close();
  };
}

const helpers = `
  window.assert = (condition, message) => { if (!condition) throw Error(message); };
  window.wait = ms => new Promise(r => setTimeout(r, ms));
  window.until = async (predicate, message = 'Timed out waiting for roster') => {
    for (let i = 0; i < 300; i++) { if (predicate()) return; await wait(10); }
    throw Error(message);
  };
  window.press = name => { const button = control(name); assert(button, 'Missing control: ' + name); button.click(); };
`;

try {
  browser("open", new URL("/privacy", origin).href);
  browser("set", "viewport", "1280", "900", "2");
  browser("wait", "main");
  // Wait for Start's document-level hydration before adding a second root.
  browser("wait", "1500");
  evaluate(`await (${fixture.toString()})(); ${helpers}
    await until(() => control('Join voice in #general')?.getAttribute('aria-disabled') === 'false', 'Join voice never became available');
    return 'Fixture mounted';`);
  console.log(
    evaluate(`
    for (let cycle = 0; cycle < 3; cycle++) {
      press('Join voice in #general'); await until(connected, 'Join did not connect');
      assert(roster().includes('Grok'), 'joined roster missing self');
      press('Leave voice'); await until(() => fixture.leavePending, 'Leave was not sent');
      // Force the exact bug: idle roster reads before the leave transaction commits.
      // Leaving ends your call card at once; the room still counts you until it commits.
      await until(() => !mount.querySelector('.live-call') && inVoice() === 1);
      const started = performance.now(); fixture.commitLeave();
      await until(() => inVoice() === 0);
      assert(performance.now() - started < 1000, 'leave waited for polling');
    }
    assert(fixture.mediaFetches === 0, 'media control or rosters bypassed the gateway');
    press('Join voice in #general'); await until(connected, 'Rejoin did not connect');
    const peer = fixture.peers.at(-1), peerCount = fixture.peers.length;
    const self = fixture.people.find(p => p.id === 'self');
    // Watch the real page throughout remote churn, not only its final state.
    let interrupted = false;
    const observer = new MutationObserver(() => { if (!connected()) interrupted = true; });
    observer.observe(mount, { subtree: true, childList: true, characterData: true, attributes: true });
    fixture.failClose = true;
    for (let cycle = 0; cycle < 3; cycle++) {
      const before = fixture.subscribes;
      pushRoster([self, { id: 'phone', name: 'Phone', muted: true, deafened: cycle % 2 === 0, tracks: [{ id: 'phone-' + cycle, kind: 'microphone' }] }]);
      await until(() => roster().includes('Phone') && fixture.subscribes > before, 'remote track was not subscribed');
      assert(row('Phone') === (cycle % 2 === 0 ? 'Phone, deafened' : 'Phone, muted'), 'first phone roster has incorrect control state');
      const closes = fixture.closes;
      pushRoster([self]); await until(() => !roster().includes('Phone') && fixture.closes > closes, 'remote track was not closed');
      await wait(30);
      assert(fixture.peers.length === peerCount && peer.connectionState === 'connected', 'remote leave replaced the local peer');
    }
    observer.disconnect(); assert(!interrupted, 'remote leave interrupted the connected call');
    assert(peer.senders[0].track.enabled, 'local microphone stopped');
    press('Deafen audio'); await until(() => control('Undeafen audio'));
    press('Leave voice'); await until(() => fixture.leavePending); fixture.commitLeave();
    await until(() => control('Join voice in #general')?.getAttribute('aria-disabled') === 'false');
    press('Join voice in #general'); await until(connected);
    assert(fixture.joinIntent.muted === true && fixture.joinIntent.deafened === true, 'rejoin did not carry remembered intent');
    press('Leave voice'); await until(() => fixture.leavePending); fixture.commitLeave();
    await until(() => !roster().length && inVoice() === 0);
    // A room you are not in shows how many are in voice, not who is muted.
    pushRoster([{ id: 'other', name: 'Other guest', muted: true, deafened: false, tracks: [] }]);
    await until(() => inVoice() === 1, 'spectator join missing');
    pushRoster([]); await until(() => inVoice() === 0, 'spectator leave missing');
    return 'PASS local leave races, remote departure with failed cleanup keeps the call and peer, rejoin mute/deafen, spectator changes';
  `),
  );
  if (artifacts) {
    mkdirSync(".amp/in/artifacts", { recursive: true });
    browser("screenshot", `${process.cwd()}/.amp/in/artifacts/live-presence-after-leave.png`);
  }
  console.log(
    evaluate(`
    // Hold the reconnection after a gateway loss, then verify a fresh snapshot replaces stale data.
    pushRoster([{ id: 'other', name: 'Other guest', muted: false, deafened: false, tracks: [] }]);
    await until(() => inVoice() === 1);
    fixture.holdSockets = true; dropSockets();
    await until(() => fixture.held.length > 0, 'gateway did not reconnect');
    // The room changed while this page was offline (to two people, so the count tells the snapshots apart).
    fixture.people = [
      { id: 'third', name: 'New guest', muted: false, deafened: false, tracks: [] },
      { id: 'fourth', name: 'Another guest', muted: false, deafened: false, tracks: [] },
    ];
    fixture.revision++;
    releaseSockets();
    await until(() => inVoice() === 2, 'reconnection kept the stale roster');
    return 'PASS reconnection replaces stale roster with the current snapshot';
  `),
  );
  console.log(
    evaluate(`
    press('Join voice in #general'); await until(connected);
    const peer = fixture.peers.at(-1), count = fixture.peers.length;
    const bounds = () => ['.chat-heading', '.chat-composer', '.voice-panel'].map(selector => mount.querySelector(selector).getBoundingClientRect().top);
    const before = bounds();
    fixture.holdSockets = true; dropSockets();
    await until(() => fixture.held.length > 0, 'gateway did not reconnect');
    assert(!mount.textContent.includes('Reconnecting live updates'), 'removed banner appeared');
    assert(JSON.stringify(bounds()) === JSON.stringify(before), 'live-update recovery shifted the layout');
    assert(connected(), 'live-update loss changed the call phase');
    releaseSockets();
    const self = fixture.people.find(p => p.id === 'self');
    for (const [muted, deafened] of [[true, false], [true, true], [false, false]]) {
      pushRoster([self, { id: 'phone', name: 'Phone', muted, deafened, tracks: [] }]);
      await until(() => row('Phone') === (deafened ? 'Phone, deafened' : muted ? 'Phone, muted' : 'Phone'), 'mute/deafen push did not arrive after recovery');
    }
    assert(peer.connectionState === 'connected' && fixture.peers.length === count && connected(), 'live-update recovery replaced the voice peer');
    assert(fixture.mediaFetches === 0, 'media control or rosters bypassed the gateway');
    await cleanup();
    return 'PASS live-update outage keeps the connected layout and voice peer; mute/deafen pushes resume (mocked gateway/WebRTC)';
  `),
  );
} catch (error) {
  console.error(
    evaluate(
      `return {page: document.body.innerText.slice(0, 2000), mediaFetches: window.fixture?.mediaFetches, phase: window.fixture?.client?.phase};`,
    ),
  );
  throw error;
} finally {
  browser("close");
}
