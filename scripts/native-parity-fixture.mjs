/** Loopback-only, disposable UI fixture. Never proxies to Caper or an SFU. */
import { createServer } from 'node:http';
import { createHash, randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';

export const fixtureIDs = {
  owner: 'owner0000001', member: 'member000001', other: 'member000002',
  space: 'space0000001', general: 'chan00000001', design: 'chan00000002', private: 'chan00000003',
  demoSpace: 'demo00000001', demo: 'demo00000002',
};
const ids = fixtureIDs;
const limits = { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 };
const initialAccount = { id: ids.owner, username: 'fixture_owner', displayName: 'Fixture Owner' };
const members = [
  { ...initialAccount, owner: true },
  { id: ids.member, username: 'maya', displayName: 'Maya', owner: false },
  { id: ids.other, username: 'alex', displayName: 'Alex', owner: false },
];
const author = (member) => ({ id: member.id, name: member.displayName, isGuest: false });
const demoSpace = { id: ids.demoSpace, name: 'Caper', ownerId: ids.owner, demo: true };
const demoChannel = { id: ids.demo, spaceId: ids.demoSpace, name: 'general', private: false };
const clone = (value) => structuredClone(value);

function initialState() {
  const account = clone(initialAccount);
  const space = { id: ids.space, name: 'Fixture Studio', ownerId: ids.owner };
  const channels = ['general', 'design', 'planning'].map((name, index) => ({
    id: [ids.general, ids.design, ids.private][index], spaceId: space.id, name, private: index === 2,
  }));
  const messages = new Map([demoChannel, ...channels].map((channel) => [channel.id, [
    ['TEST FIXTURE — local sample data, not a live conversation.', members[0]],
    ['The same conversation should feel familiar on every platform.', members[1]],
    ['Keep the space rail, channel list, and audio controls in their usual places.', members[1]],
    ['Agreed. Let’s check the narrow layout and the management dialogs too.', members[2]],
  ].map(([text, member], index) => ({
    id: `message-${channel.id}-${index + 1}`, channelId: channel.id, seq: String(index + 1),
    author: author(member), content: { version: 1, type: 'text', text },
    clientMessageId: `00000000-0000-4000-8000-00000000000${index + 1}`,
    createdAt: `2026-09-23T09:${40 + index}:00.000Z`,
  }))]));
  return {
    spaces: [{ space, channels, members: clone(members) }], messages,
    grants: new Map([[ids.private, [ids.owner, ids.member]]]), failures: [], challenges: new Map(),
    account, chatSessions: new Map(), sendKeys: new Map(), typingRevision: 0,
  };
}

function socketFrame(value, opcode = 1) {
  const body = Buffer.isBuffer(value) ? value : Buffer.from(JSON.stringify(value));
  const header = Buffer.alloc(body.length < 126 ? 2 : 4);
  header[0] = 0x80 | opcode;
  header[1] = body.length < 126 ? body.length : 126;
  if (body.length >= 126) header.writeUInt16BE(body.length, 2);
  return Buffer.concat([header, body]);
}

export async function startFixture({ port = 3001, gatewayPort = 3002 } = {}) {
  let state = initialState();
  const sockets = new Set();
  const identity = (request) => request.headers.authorization === 'Bearer fixture-owner-token'
    || /(?:^|;\s*)caper_fixture=owner(?:;|$)/.test(request.headers.cookie ?? '') ? state.account : undefined;
  const channelFor = (id) => id === ids.demo ? demoChannel
    : state.spaces.flatMap((detail) => detail.channels).find((channel) => channel.id === id);
  const spaceFor = (id) => id === ids.demoSpace ? demoSpace : state.spaces.find((detail) => detail.space.id === id)?.space;
  const broadcast = (kind, channelId, event) => {
    for (const client of sockets) for (const [id, sub] of client.subscriptions) {
      if (sub.kind === kind && (sub.channelId ?? ids.demo) === channelId) client.send({ type: 'event', id, event });
    }
  };
  const json = (response, status, body, headers = {}) => {
    response.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store', ...headers });
    response.end(status === 204 ? undefined : JSON.stringify(body));
  };
  const reject = (response, status, error) => json(response, status, { error });

  const serve = async (request, response) => {
    try {
      const url = new URL(request.url, 'http://localhost');
      const path = url.pathname.replace(/^\/api\/v1\//, '/api/');
      let bytes = 0;
      const chunks = [];
      for await (const chunk of request) {
        bytes += chunk.length;
        if (bytes > 256 * 1024) return reject(response, 413, 'Fixture request too large.');
        chunks.push(chunk);
      }
      const body = chunks.length ? JSON.parse(Buffer.concat(chunks)) : {};
      const method = request.method;
      if (path === '/health') return json(response, 200, { fixture: true });
      if (path === '/__fixture/control' && method === 'POST') {
        if (body.reset) state = initialState();
        if (body.failure) state.failures.push(body.failure);
        if (body.disconnect) for (const client of sockets) client.socket.destroy();
        if (body.typing) broadcast('chat', body.typing.channelId ?? ids.general, {
          type: 'typing.updated', channelId: body.typing.channelId ?? ids.general,
          author: author(members[1]), typing: body.typing.active !== false, revision: String(++state.typingRevision),
        });
        return json(response, 200, { fixture: true });
      }
      const failureIndex = state.failures.findIndex((failure) => failure.path === path && (!failure.method || failure.method === method));
      if (failureIndex >= 0) {
        const [failure] = state.failures.splice(failureIndex, 1);
        return reject(response, failure.status, failure.error ?? 'TEST FIXTURE: requested failure.');
      }
      const user = identity(request);
      if (path === '/api/account/me') return user ? json(response, 200, state.account) : reject(response, 401, 'Sign in required.');
      if (path === '/api/auth/email/request' && method === 'POST') {
        if (!String(body.email ?? '').includes('@')) return reject(response, 400, 'Enter a valid email address.');
        const challengeId = randomUUID(); state.challenges.set(challengeId, 3);
        return json(response, 200, { challengeId });
      }
      if (path === '/api/auth/email/verify' && method === 'POST') {
        const attempts = state.challenges.get(body.challengeId) ?? 0;
        if (!attempts || body.code !== 'ABC234') {
          state.challenges.set(body.challengeId, Math.max(0, attempts - 1));
          return json(response, 401, { error: 'Incorrect code.', attemptsRemaining: Math.max(0, attempts - 1) });
        }
        state.challenges.delete(body.challengeId);
        return json(response, 200, { account: state.account, token: 'fixture-owner-token' }, { 'set-cookie': 'caper_fixture=owner; Path=/; SameSite=Lax' });
      }
      if (path === '/api/auth/logout' && method === 'POST') return json(response, 204, undefined, { 'set-cookie': 'caper_fixture=; Max-Age=0; Path=/' });
      if (path === '/api/account/profile' && method === 'POST') {
        if (!user) return reject(response, 401, 'Sign in required.');
        const username = typeof body.username === 'string' ? body.username.trim().toLowerCase() : '';
        const displayName = typeof body.displayName === 'string' ? body.displayName.trim() : '';
        if (!/^[a-z0-9_]{3,32}$/.test(username) || !displayName || [...displayName].length > 64 || /[\p{Cc}]/u.test(displayName))
          return reject(response, 400, 'invalid profile');
        state.account = { ...state.account, username, displayName };
        for (const detail of state.spaces) {
          const owner = detail.members.find((member) => member.id === ids.owner);
          if (owner) Object.assign(owner, state.account);
        }
        for (const sessionAuthor of state.chatSessions.values()) if (!sessionAuthor.isGuest) Object.assign(sessionAuthor, author(state.account));
        return json(response, 200, state.account);
      }
      if (path === '/api/chat/session' && method === 'POST') {
        const token = `fixture-chat-${randomUUID()}`;
        const requestedName = typeof body.name === 'string' ? body.name.trim() : '';
        const name = user ? state.account.displayName : requestedName;
        if (!name || [...name].length > 64 || /[\p{Cc}]/u.test(name)) return reject(response, 400, 'invalid name');
        const who = user ? author(state.account) : { id: randomUUID().replaceAll('-', '').slice(0, 12), name, isGuest: true };
        state.chatSessions.set(token, who);
        return json(response, 200, { token, author: who });
      }
      if (/^\/api\/(?:channels\/[^/]+\/)?media\/status$/.test(path)) return json(response, 200, { enabled: true });
      if (/^\/api\/(?:channels\/[^/]+\/)?media\//.test(path)) return reject(response, 503, 'TEST FIXTURE: no real media engine or SFU is connected.');
      const chat = /^\/api\/chat\/channels\/([^/]+)\/messages$/.exec(path);
      if (path === '/api/chat/general' || chat) {
        const channel = channelFor(chat?.[1] ?? ids.demo);
        if (!channel) return reject(response, 404, 'Channel not found.');
        if (channel.id !== ids.demo && !user) return reject(response, 401, 'Sign in required.');
        const messages = state.messages.get(channel.id) ?? [];
        if (method === 'POST') {
          const who = state.chatSessions.get(request.headers['x-caper-chat-token']);
          if (!who) return reject(response, 401, 'Messaging session required.');
          if (typeof body.clientMessageId !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(body.clientMessageId)
            || typeof body.text !== 'string' || !body.text.trim() || [...body.text].length > 4000 || /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(body.text))
            return reject(response, 400, 'Enter a message of at most 4,000 characters.');
          const key = `${channel.id}:${body.clientMessageId}`;
          const previous = state.sendKeys.get(key);
          if (previous) return previous.token === request.headers['x-caper-chat-token'] && previous.text === body.text
            ? json(response, 200, previous.message) : reject(response, 409, 'Message ID already used.');
          const message = { id: randomUUID(), channelId: channel.id, seq: String(messages.length + 1), author: who,
            content: { version: 1, type: 'text', text: body.text }, createdAt: new Date().toISOString(), clientMessageId: body.clientMessageId };
          messages.push(message); state.messages.set(channel.id, messages);
          state.sendKeys.set(key, { token: request.headers['x-caper-chat-token'], text: body.text, message });
          broadcast('chat', channel.id, { type: 'message.created', channelId: channel.id, seq: message.seq, message });
          return json(response, 200, message);
        }
        const before = url.searchParams.get('before');
        if (before !== null && !/^(0|[1-9]\d*)$/.test(before)) return reject(response, 400, 'invalid cursor');
        const available = before !== null ? messages.filter((message) => BigInt(message.seq) < BigInt(before)) : messages;
        return json(response, 200, { space: spaceFor(channel.spaceId), channel, messages: available.slice(-50), cursor: messages.at(-1)?.seq ?? '0', hasMore: available.length > 50 });
      }
      const spacePath = /^\/api\/spaces(?:\/([^/]+))?(?:\/channels\/([^/]+))?(?:\/(channels|members)(?:\/([^/]+))?)?$/.exec(path);
      if (spacePath) {
        if (!user) return reject(response, 401, 'Sign in required.');
        const [, spaceId, channelId, section, memberId] = spacePath;
        if (!spaceId) {
          if (method === 'GET') return json(response, 200, { spaces: state.spaces.map((detail) => detail.space), limits });
          if (method === 'POST') {
            const name = typeof body.name === 'string' ? body.name.trim() : '';
            if (!name || [...name].length > 80 || /[\p{Cc}]/u.test(name)) return reject(response, 400, 'invalid space name');
            const id = randomUUID().replaceAll('-', '').slice(0, 12);
            const space = { id, name, ownerId: state.account.id };
            const channel = { id: randomUUID().replaceAll('-', '').slice(0, 12), spaceId: id, name: 'general', private: false };
            state.spaces.push({ space, channels: [channel], members: [{ ...state.account, owner: true }] }); state.messages.set(channel.id, []);
            return json(response, 201, space);
          }
        }
        const detail = state.spaces.find((entry) => entry.space.id === spaceId);
        if (!detail) return reject(response, 404, 'Space not found.');
        const channel = detail.channels.find((entry) => entry.id === channelId);
        if (channelId && !channel) return reject(response, 404, 'Channel not found.');
        if (section === 'members') {
          const list = channelId ? detail.members.filter((member) => (state.grants.get(channelId) ?? []).includes(member.id)) : detail.members;
          if (method === 'GET') return json(response, 200, { members: list });
          if (method === 'POST') {
            const member = members.find((entry) => entry.username === body.username);
            if (!member) return reject(response, 404, 'No account has that username.');
            if (channelId && !detail.members.some((entry) => entry.id === member.id)) return reject(response, 404, 'Member not found.');
            const exists = channelId ? (state.grants.get(channelId) ?? []).includes(member.id) : detail.members.some((entry) => entry.id === member.id);
            if (channelId) state.grants.set(channelId, [...new Set([...(state.grants.get(channelId) ?? []), member.id])]);
            else if (!exists) detail.members.push(clone(member));
            return json(response, exists ? 200 : 201, member);
          }
          if (method === 'DELETE') {
            if (memberId === state.account.id) return reject(response, 409, 'The owner cannot be removed.');
            const exists = channelId ? (state.grants.get(channelId) ?? []).includes(memberId) : detail.members.some((member) => member.id === memberId);
            if (!exists) return reject(response, 404, 'Member not found.');
            if (channelId) state.grants.set(channelId, (state.grants.get(channelId) ?? []).filter((id) => id !== memberId));
            else {
              detail.members = detail.members.filter((member) => member.id !== memberId);
              for (const item of detail.channels) state.grants.set(item.id, (state.grants.get(item.id) ?? []).filter((id) => id !== memberId));
            }
            return json(response, 204);
          }
        }
        if (section === 'channels' && method === 'POST') {
          if (!/^[a-z]+(?:-[a-z]+)*$/.test(body.name ?? '') || body.name.length > 80) return reject(response, 400, 'Use lowercase letters separated by single dashes.');
          if (detail.channels.some((entry) => entry.name === body.name)) return reject(response, 409, 'channel name already exists');
          const next = { id: randomUUID().replaceAll('-', '').slice(0, 12), spaceId, name: body.name, private: !!body.private };
          detail.channels.push(next); state.messages.set(next.id, []); state.grants.set(next.id, [state.account.id]);
          return json(response, 201, next);
        }
        if (method === 'PATCH') {
          if (channel) {
            if (!/^[a-z]+(?:-[a-z]+)*$/.test(body.name ?? '') || body.name.length > 80) return reject(response, 400, 'invalid channel name');
            if (detail.channels.some((entry) => entry !== channel && entry.name === body.name)) return reject(response, 409, 'channel name already exists');
            Object.assign(channel, { name: body.name, private: !!body.private });
          } else {
            const name = typeof body.name === 'string' ? body.name.trim() : '';
            if (!name || [...name].length > 80 || /[\p{Cc}]/u.test(name)) return reject(response, 400, 'invalid space name');
            detail.space.name = name;
          }
          return json(response, 200, channel ?? detail.space);
        }
        if (method === 'DELETE') {
          if (channel) detail.channels = detail.channels.filter((entry) => entry.id !== channelId);
          else state.spaces = state.spaces.filter((entry) => entry.space.id !== spaceId);
          return json(response, 204);
        }
        if (method === 'GET') return json(response, 200, channel ?? detail);
      }
      reject(response, 404, `TEST FIXTURE: unsupported ${method} ${path}`);
    } catch { reject(response, 400, 'TEST FIXTURE: malformed request.'); }
  };

  const upgrade = (request, socket, head) => {
    if (request.url !== '/api/chat/events' || !request.headers['sec-websocket-key']) return socket.destroy();
    const accept = createHash('sha1').update(`${request.headers['sec-websocket-key']}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const client = { socket, subscriptions: new Map(), send: (value) => { if (!socket.destroyed) socket.write(socketFrame(value)); } };
    sockets.add(client);
    client.send({ type: 'hello', serverTime: Date.now(), idleTimeoutSeconds: 30 });
    const heartbeat = setInterval(() => client.send({ type: 'heartbeat' }), 5000);
    socket.on('error', () => {});
    socket.on('close', () => { clearInterval(heartbeat); sockets.delete(client); });
    const receive = (frame) => {
      if (frame.type === 'subscribe') {
        if (frame.kind === 'chat') {
          const channelId = frame.channelId ?? ids.demo;
          const messages = state.messages.get(channelId);
          if (!messages || !/^(0|[1-9]\d*)$/.test(frame.after ?? '0')) return client.send({ type: 'error', id: frame.id, status: 400, error: 'invalid subscription' });
          client.subscriptions.set(frame.id, frame);
          for (const message of messages.filter((message) => BigInt(message.seq) > BigInt(frame.after ?? '0')))
            client.send({ type: 'event', id: frame.id, event: { type: 'message.created', channelId: message.channelId, seq: message.seq, message } });
          client.send({ type: 'event', id: frame.id, event: { type: 'ready', cursor: messages.at(-1)?.seq ?? '0' } });
        } else if (frame.kind === 'presence') {
          if (!frame.spaceId || !Array.isArray(frame.userIds) || !frame.userIds.length || frame.userIds.length > 100)
            return client.send({ type: 'error', id: frame.id, status: 400, error: 'invalid subscription' });
          const detail = state.spaces.find((entry) => entry.space.id === frame.spaceId);
          if (!detail || frame.userIds.some((userId) => !detail.members.some((member) => member.id === userId)))
            return client.send({ type: 'error', id: frame.id, status: 403, error: 'presence subscription denied' });
          client.subscriptions.set(frame.id, frame);
          client.send({ type: 'event', id: frame.id, event: {
            type: 'snapshot', members: frame.userIds.map((userId) => ({ userId, status: userId === ids.other ? 'idle' : 'online' })),
          } });
        } else if (frame.kind === 'media') {
          client.subscriptions.set(frame.id, frame);
          client.send({ type: 'event', id: frame.id, event: { type: 'snapshot', revision: 1, participants: [] } });
        } else return client.send({ type: 'error', id: frame.id, status: 400, error: 'invalid subscription' });
        client.send({ type: 'subscribed', id: frame.id });
      } else if (frame.type === 'unsubscribe') client.subscriptions.delete(frame.id);
      else if (frame.type === 'command') {
        if (frame.method === 'typing') {
          const who = state.chatSessions.get(frame.chatToken);
          const channel = channelFor(frame.channelId);
          if (!who || !channel || typeof frame.body?.typing !== 'boolean') return client.send({ type: 'result', id: frame.id, status: 401, body: { error: 'guest session expired' } });
          broadcast('chat', channel.id, { type: 'typing.updated', channelId: channel.id, author: who,
            typing: frame.body.typing, revision: String(++state.typingRevision) });
          client.send({ type: 'result', id: frame.id, status: 204, body: {} });
        } else client.send({ type: 'result', id: frame.id, status: 503, body: { error: 'TEST FIXTURE: no live SFU.' } });
      }
      else if (frame.type === 'heartbeat') client.send({ type: 'heartbeat' });
    };
    let pending = Buffer.alloc(0);
    const data = (chunk) => {
      pending = Buffer.concat([pending, chunk]);
      try {
        while (pending.length >= 2) {
          const opcode = pending[0] & 15;
          if (!(pending[0] & 128) || !(pending[1] & 128) || (pending[1] & 127) === 127) return socket.destroy();
          const extended = (pending[1] & 127) === 126;
          if (pending.length < (extended ? 8 : 6)) return;
          const length = extended ? pending.readUInt16BE(2) : pending[1] & 127;
          const offset = extended ? 4 : 2;
          if (pending.length < offset + 4 + length) return;
          const mask = pending.subarray(offset, offset + 4);
          const payload = Buffer.from(pending.subarray(offset + 4, offset + 4 + length));
          for (let index = 0; index < payload.length; index++) payload[index] ^= mask[index % 4];
          pending = pending.subarray(offset + 4 + length);
          if (opcode === 8) return socket.end(socketFrame(Buffer.alloc(0), 8));
          if (opcode === 9) socket.write(socketFrame(payload, 10));
          else if (opcode === 1) receive(JSON.parse(payload));
          else if (opcode !== 10) return socket.destroy();
        }
      } catch { socket.destroy(); }
    };
    socket.on('data', data);
    if (head.length) data(head);
  };
  const servers = [...new Set([port, gatewayPort])].map(() => createServer(serve));
  for (let index = 0; index < servers.length; index++) {
    servers[index].on('upgrade', upgrade);
    await new Promise((resolve, reject) => {
      servers[index].once('error', reject);
      servers[index].listen(index === 0 ? port : gatewayPort, '127.0.0.1', resolve);
    });
  }
  return {
    port: servers[0].address().port,
    gatewayPort: servers.at(-1).address().port,
    async close() { for (const client of sockets) client.socket.destroy(); await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve)))); },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const fixture = await startFixture({ port: Number(process.env.CAPER_FIXTURE_PORT ?? 3001), gatewayPort: Number(process.env.CAPER_FIXTURE_GATEWAY_PORT ?? 3002) });
  console.log(`TEST FIXTURE ONLY: local mock API on port ${fixture.port}; code ABC234; no real email or SFU.`);
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => void fixture.close().then(() => process.exit(0)));
}
