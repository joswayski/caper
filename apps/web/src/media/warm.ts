import { appGateway } from "../gateway/client.ts";

/**
 * Two provider sessions a signed-in member's browser creates and connects
 * before any Join, with no tracks: one to publish into, one to pull into.
 * Cloudflare keeps a connected session without media, so Join can publish and
 * pull straight away, skipping session creation, TURN issuance and both
 * connection handshakes. The ticket is signed for this account and works in
 * any of its channels.
 */
export interface WarmVoice {
  ticket: string;
  main: RTCPeerConnection;
  receive: RTCPeerConnection;
  iceServers: RTCIceServer[];
  /** performance.now() when both connected. */
  connectedAt: number;
}

interface WarmResponse {
  ticket: string;
  main: RTCSessionDescriptionInit;
  receive: RTCSessionDescriptionInit;
  iceServers: RTCIceServer[];
  adoptWithinMs: number;
}

/** Replace well before the API stops accepting the ticket (25 minutes). */
const REPLACE_AFTER_MS = 20 * 60_000;
const CONNECT_TIMEOUT_MS = 12_000;
/** A disconnected connection gets this long to recover before being replaced. */
const DISCONNECT_GRACE_MS = 5_000;
const RETRY_DELAYS_MS = [2_000, 5_000, 15_000, 60_000];

let root: string | undefined;
let current: WarmVoice | undefined;
let creating = false;
let failures = 0;
let unsupported = false;
let timer: ReturnType<typeof setTimeout> | undefined;
let detach: (() => void) | undefined;
let visibilityListening = false;

type WarmRequest = (channelId: string | undefined, body: object) => Promise<WarmResponse>;
const gatewayRequest: WarmRequest = (channelId, body) =>
  appGateway().command({ method: "media.warm", channelId, body, timeoutMs: 10_000 }) as Promise<WarmResponse>;
let request = gatewayRequest;

/** Replaces the API call and resets this module's state. */
export function setWarmRequestForTests(replacement?: WarmRequest) {
  stopVoiceWarm();
  request = replacement ?? gatewayRequest;
  unsupported = false;
  failures = 0;
}

function channelFromRoot(apiRoot: string) {
  const match = /^\/api\/channels\/([^/]+)\/media$/.exec(apiRoot);
  return match ? decodeURIComponent(match[1]) : undefined;
}

function healthy(warm: WarmVoice) {
  return warm.main.connectionState === "connected" && warm.receive.connectionState === "connected"
    && performance.now() - warm.connectedAt < REPLACE_AFTER_MS;
}

export function closeWarmVoice(warm: WarmVoice) {
  warm.main.close();
  warm.receive.close();
}

function discard() {
  detach?.();
  detach = undefined;
  if (current) closeWarmVoice(current);
  current = undefined;
}

function schedule(delay: number) {
  clearTimeout(timer);
  timer = setTimeout(() => void refresh(), delay);
}

/**
 * Keeps one warm pair for this signed-in page while voice is idle. Idempotent;
 * `apiRoot` is any of the member's account channels (the public demo has none).
 */
export function keepVoiceWarm(apiRoot: string) {
  if (typeof RTCPeerConnection === "undefined" || !channelFromRoot(apiRoot)) return;
  root = apiRoot;
  if (!visibilityListening && typeof document !== "undefined") {
    visibilityListening = true;
    // A backgrounded phone suspends the page and its connections; rebuild on return.
    document.addEventListener("visibilitychange", () => { if (root && document.visibilityState === "visible") void refresh(); });
  }
  void refresh();
}

/** Closes the warm pair and stops replacing it (sign-out, leaving the page, or a call). */
export function stopVoiceWarm() {
  root = undefined;
  clearTimeout(timer);
  discard();
}

export function hasWarmVoice() {
  return !!current && healthy(current);
}

/** Transfers a healthy pair to a joining call, which then owns and closes it. */
export function takeWarmVoice(): WarmVoice | undefined {
  if (!current || !healthy(current)) return undefined;
  const warm = current;
  detach?.();
  detach = undefined;
  current = undefined;
  return warm;
}

async function refresh() {
  if (!root || unsupported || creating) return;
  if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
  if (current && healthy(current)) {
    schedule(REPLACE_AFTER_MS - (performance.now() - current.connectedAt));
    return;
  }
  const apiRoot = root;
  creating = true;
  try {
    const warm = await create(apiRoot);
    if (root !== apiRoot) { closeWarmVoice(warm); return; }
    discard();
    current = warm;
    detach = watch(warm);
    failures = 0;
    schedule(REPLACE_AFTER_MS);
  } catch (error) {
    const status = (error as { status?: number })?.status;
    // An API without warm sessions answers 404 (or rejects the operation): stop asking.
    if (status === 404 || status === 400) { unsupported = true; return; }
    schedule(RETRY_DELAYS_MS[Math.min(failures++, RETRY_DELAYS_MS.length - 1)]);
  } finally {
    creating = false;
  }
}

function watch(warm: WarmVoice) {
  let grace: ReturnType<typeof setTimeout> | undefined;
  const changed = () => {
    if (current !== warm) return;
    const states = [warm.main.connectionState, warm.receive.connectionState];
    if (states.every((state) => state === "connected")) { clearTimeout(grace); grace = undefined; return; }
    if (states.some((state) => state === "failed" || state === "closed")) { discard(); schedule(0); return; }
    grace ??= setTimeout(() => { if (current === warm && !healthy(warm)) { discard(); schedule(0); } }, DISCONNECT_GRACE_MS);
  };
  warm.main.addEventListener("connectionstatechange", changed);
  warm.receive.addEventListener("connectionstatechange", changed);
  return () => {
    clearTimeout(grace);
    warm.main.removeEventListener("connectionstatechange", changed);
    warm.receive.removeEventListener("connectionstatechange", changed);
  };
}

async function create(apiRoot: string): Promise<WarmVoice> {
  const main = new RTCPeerConnection({ bundlePolicy: "max-bundle" });
  const receive = new RTCPeerConnection({ bundlePolicy: "max-bundle" });
  try {
    // A negotiated data channel gives each connection a transport without any
    // track and without in-band channel messages. Join adds the audio.
    for (const pc of [main, receive]) pc.createDataChannel("warm", { negotiated: true, id: 0 });
    const [mainOffer, receiveOffer] = await Promise.all([main.createOffer(), receive.createOffer()]);
    const response = await request(channelFromRoot(apiRoot), {
      main: { type: "offer", sdp: mainOffer.sdp }, receive: { type: "offer", sdp: receiveOffer.sdp },
    });
    // TURN before gathering starts, as Join does.
    const connect = async (pc: RTCPeerConnection, offer: RTCSessionDescriptionInit, answer: RTCSessionDescriptionInit) => {
      pc.setConfiguration({ ...pc.getConfiguration(), iceServers: response.iceServers });
      await pc.setLocalDescription(offer);
      await pc.setRemoteDescription(answer);
      await connected(pc);
    };
    await Promise.all([connect(main, mainOffer, response.main), connect(receive, receiveOffer, response.receive)]);
    return { ticket: response.ticket, main, receive, iceServers: response.iceServers, connectedAt: performance.now() };
  } catch (error) {
    main.close();
    receive.close();
    throw error;
  }
}

function connected(pc: RTCPeerConnection) {
  if (pc.connectionState === "connected") return Promise.resolve();
  return new Promise<void>((resolve, reject) => {
    const done = (error?: Error) => {
      clearTimeout(deadline);
      pc.removeEventListener("connectionstatechange", changed);
      if (error) reject(error); else resolve();
    };
    const changed = () => {
      if (pc.connectionState === "connected") done();
      else if (pc.connectionState === "failed" || pc.connectionState === "closed") done(new Error("Warm connection failed."));
    };
    const deadline = setTimeout(() => done(new Error("Warm connection timed out.")), CONNECT_TIMEOUT_MS);
    pc.addEventListener("connectionstatechange", changed);
  });
}
