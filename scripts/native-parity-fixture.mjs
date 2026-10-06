/** Loopback-only, disposable UI fixture. Never proxies to Caper or an SFU. */
import { createServer } from 'node:http';
import { createHash, randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';

export const fixtureIDs = {
  owner: 'owner0000001', member: 'member000001', other: 'member000002',
  invitee: 'invitee00001',
  space: 'space0000001', general: 'chan00000001', design: 'chan00000002', private: 'chan00000003',
  demoSpace: 'demo00000001', demo: 'demo00000002',
  direct: 'dm0000000001', selfDirect: 'dm0000000002',
};
const ids = fixtureIDs;
const limits = { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 };
const initialAccount = { id: ids.owner, username: 'fixture_owner', displayName: 'Fixture Owner', avatarId: 0 };
const members = [
  { ...initialAccount, owner: true },
  { id: ids.member, username: 'maya', displayName: 'Maya', owner: false, avatarId: 31 },
  { id: ids.other, username: 'alex', displayName: 'Alex', owner: false, avatarId: 799 },
];
const accounts = [...members,
  { id: ids.invitee, username: 'sam', displayName: 'Sam', owner: false, avatarId: 719 },
];
const author = (member) => ({ id: member.id, name: member.displayName, isGuest: false, avatarId: member.avatarId });
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
    // Same shape as the API's message IDs (15 ASCII alphanumerics); native
    // clients refuse to build reaction paths for anything else.
    id: `${channel.id}m${String(index + 1).padStart(2, '0')}`, channelId: channel.id, seq: String(index + 1),
    author: author(member), content: { version: 1, type: 'text', text },
    clientMessageId: `00000000-0000-4000-8000-00000000000${index + 1}`,
    createdAt: `2026-09-23T09:${40 + index}:00.000Z`,
  }))]));
  return {
    spaces: [{ space, channels, members: clone(members) }], messages, directs: [],
    reactionEvents: new Map(),
    invitations: new Map(),
    channelInvitations: new Map(),
    joins: new Map(channels.map(channel => [channel.id, channel.private ? [ids.owner, ids.member] : members.map(member => member.id)])),
    grants: new Map([[ids.private, [ids.owner, ids.member]]]), failures: [], challenges: new Map(),
    account, chatSessions: new Map(), sendKeys: new Map(), typingRevision: 0,
    media: new Map([[ids.design, { type: 'snapshot', revision: 1, sessionStartedAt: Date.now() - 1_701_000, participants: [
      { id: 'fixture-voice-maya', name: 'TEST FIXTURE Maya', muted: false, deafened: false, avatarId: 31 },
      { id: 'fixture-voice-alex', name: 'TEST FIXTURE Alex', muted: true, deafened: false, avatarId: 799 },
    ] }]]),
    mediaDenied: new Set(),
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
    || /(?:^|;\s*)caper_fixture=owner(?:;|$)/.test(request.headers.cookie ?? '') ? state.account
    : request.headers.authorization === 'Bearer fixture-member-token' ? members[1] : undefined;
  const channelFor = (id) => id === ids.demo ? demoChannel
    : state.directs.some((conversation) => conversation.id === id) ? { id, spaceId: '', name: state.directs.find(conversation => conversation.id === id).peer.displayName, private: true, direct: true }
    : state.spaces.flatMap((detail) => detail.channels).find((channel) => channel.id === id);
  const spaceFor = (id) => id === '' ? { id: '', name: 'Direct messages' } : id === ids.demoSpace ? demoSpace : state.spaces.find((detail) => detail.space.id === id)?.space;
  const canRead = (channel, user) => channel?.id === ids.demo || !!user && (channel?.direct
    ? user.id === state.account.id && state.directs.some(conversation => conversation.id === channel.id)
    : state.spaces.some(detail => detail.space.id === channel?.spaceId && detail.members.some(member => member.id === user.id)
      && (!channel.private || detail.space.ownerId === user.id || (state.grants.get(channel.id) ?? []).includes(user.id))));
  const canParticipate = (channel, user) => canRead(channel, user)
    && (channel.direct || channel.id === ids.demo || (state.joins.get(channel.id) ?? []).includes(user.id));
  const channelDTO = (channel, user) => ({ ...channel, joined: !!channel.direct || channel.id === ids.demo || (state.joins.get(channel.id) ?? []).includes(user?.id) });
  const broadcast = (kind, channelId, event) => {
    for (const client of sockets) for (const [id, sub] of client.subscriptions) {
      if (sub.kind === kind && (sub.channelId ?? ids.demo) === channelId) client.send({ type: 'event', id, event });
    }
  };
  const channelEvents = (channelId) => [
    ...(state.messages.get(channelId) ?? []).map(message => ({ type: 'message.created', channelId, seq: message.seq, message })),
    ...(state.reactionEvents.get(channelId) ?? []),
  ].sort((a, b) => BigInt(a.seq) < BigInt(b.seq) ? -1 : 1);
  const channelHead = (channelId) => channelEvents(channelId).at(-1)?.seq ?? '0';
  const react = (channelId, message, emoji, userId, active) => {
    const reactions = message.reactions ?? [];
    const own = reactions.find(reaction => reaction.emoji === emoji)?.authorIds.includes(userId) ?? false;
    if (own !== active) {
      const reaction = reactions.find(reaction => reaction.emoji === emoji) ?? { emoji, authorIds: [] };
      if (!reactions.includes(reaction)) reactions.push(reaction);
      reaction.authorIds = active ? [...reaction.authorIds, userId] : reaction.authorIds.filter(id => id !== userId);
      message.reactions = reactions.filter(reaction => reaction.authorIds.length);
      message.reactionSeq = String(BigInt(channelHead(channelId)) + 1n);
      const event = { type: 'message.reactions', schemaVersion: 1, channelId, seq: message.reactionSeq, messageId: message.id, reactions: clone(message.reactions) };
      state.reactionEvents.set(channelId, [...(state.reactionEvents.get(channelId) ?? []), event]);
      broadcast('chat', channelId, event);
    }
    return { type: 'message.reactions', schemaVersion: 1, channelId, seq: message.reactionSeq ?? '0', messageId: message.id, reactions: message.reactions ?? [] };
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
        if (body.noSpaces) state.spaces = [];
        if (body.clearFailures) state.failures = [];
        if (body.failure) state.failures.push(body.failure);
        if (body.disconnect) for (const client of sockets) client.socket.destroy();
        if (body.typing) broadcast('chat', body.typing.channelId ?? ids.general, {
          type: 'typing.updated', channelId: body.typing.channelId ?? ids.general,
          author: author(members[1]), typing: body.typing.active !== false, revision: String(++state.typingRevision),
        });
        if (body.incomingMessage) {
          const channelId = body.incomingMessage.channelId;
          if (!channelFor(channelId)) return reject(response, 404, 'Fixture channel not found.');
          const messages = state.messages.get(channelId) ?? [];
          const message = {
            id: `fixture${String(messages.length + 1).padStart(8, '0')}`, channelId, seq: String(BigInt(channelHead(channelId)) + 1n),
            author: author(members[2]), content: { version: 1, type: 'text', text: `TEST FIXTURE — ${body.incomingMessage.text}` },
            clientMessageId: randomUUID(), createdAt: new Date().toISOString(),
          };
          messages.push(message); state.messages.set(channelId, messages);
          broadcast('chat', channelId, { type: 'message.created', channelId, seq: message.seq, message });
        }
        if (body.incomingReaction) {
          const { channelId, messageId, emoji, active = true, userId = ids.other } = body.incomingReaction;
          const message = state.messages.get(channelId)?.find(message => message.id === messageId);
          if (!message) return reject(response, 404, 'Fixture message not found.');
          if (!accounts.some(account => account.id === userId)) return reject(response, 400, 'Unknown fixture account.');
          react(channelId, message, emoji, userId, active);
        }
        if (body.media) {
          const channelId = body.media.channelId ?? ids.demo;
          if (!channelFor(channelId) || !Array.isArray(body.media.participants)) return reject(response, 400, 'invalid media fixture');
          const previous = state.media.get(channelId);
          const snapshot = { type: 'snapshot', revision: (state.media.get(channelId)?.revision ?? 0) + 1,
            sessionStartedAt: body.media.participants.length
              ? body.media.sessionStartedAt ?? previous?.sessionStartedAt ?? Date.now() : null,
            participants: body.media.participants.map(({ id, name, muted, deafened }) => ({ id, name, muted, deafened })) };
          state.media.set(channelId, snapshot);
          broadcast('media', channelId, snapshot);
        }
        if (body.mediaAccessDenied) {
          const channelId = body.mediaAccessDenied.channelId ?? ids.demo;
          if (body.mediaAccessDenied.denied === false) state.mediaDenied.delete(channelId);
          else {
            state.mediaDenied.add(channelId);
            for (const client of sockets) for (const [id, sub] of client.subscriptions) {
              if (sub.kind === 'media' && (sub.channelId ?? ids.demo) === channelId) {
                client.subscriptions.delete(id);
                client.send({ type: 'error', id, status: 403, error: 'TEST FIXTURE: voice access ended.' });
              }
            }
          }
        }
        return json(response, 200, { fixture: true });
      }
      const failureIndex = state.failures.findIndex((failure) => failure.path === path && (!failure.method || failure.method === method));
      if (failureIndex >= 0) {
        const failure = state.failures[failureIndex];
        if (!failure.persistent) state.failures.splice(failureIndex, 1);
        return reject(response, failure.status, failure.error ?? 'TEST FIXTURE: requested failure.');
      }
      const user = identity(request);
      if (path === '/api/push/config') return user ? json(response, 200, { platforms: [] }) : reject(response, 401, 'Sign in required.');
      if (path === '/api/dms') {
        if (!user) return reject(response, 401, 'Sign in required.');
        if (method === 'POST') {
          const username = String(body.username).trim().replace(/^@/, '').toLowerCase();
          const self = username === state.account.username;
          if (!self && !['fixture_alex', 'alex'].includes(username)) return reject(response, 404, 'Account not found.');
          const id = self ? ids.selfDirect : ids.direct;
          if (!state.directs.some(conversation => conversation.id === id)) {
            const peer = self ? { id: state.account.id, username: state.account.username, displayName: state.account.displayName }
              : { id: ids.other, username: 'fixture_alex', displayName: 'TEST FIXTURE Alex' };
            state.directs.push({ id, peer, lastSeq: '0', readSeq: '0' });
            state.messages.set(id, []);
          }
          return json(response, 200, state.directs.find(conversation => conversation.id === id));
        }
        return json(response, 200, { conversations: user.id === state.account.id ? state.directs.map((conversation) => ({ ...conversation, lastSeq: channelHead(conversation.id) })) : [] });
      }
      const directRead = /^\/api\/dms\/([^/]+)\/read$/.exec(path);
      if (directRead && method === 'POST') {
        if (!user) return reject(response, 401, 'Sign in required.');
        const conversation = state.directs.find((item) => item.id === directRead[1]);
        if (!conversation) return reject(response, 404, 'Conversation not found.');
        if (!/^(0|[1-9]\d*)$/.test(body.seq)) return reject(response, 400, 'Invalid cursor.');
        const head = BigInt(channelHead(conversation.id));
        const bounded = BigInt(body.seq) > head ? head : BigInt(body.seq);
        if (bounded > BigInt(conversation.readSeq)) conversation.readSeq = String(bounded);
        return json(response, 204);
      }
      if (path === '/api/account/me') return user ? json(response, 200, user) : reject(response, 401, 'Sign in required.');
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
      const mediaChannel = /^\/api\/channels\/([^/]+)\/media\//.exec(path);
      if (mediaChannel && !canParticipate(channelFor(mediaChannel[1]), user)) return reject(response, 404, 'resource not found');
      if (/^\/api\/(?:channels\/[^/]+\/)?media\/status$/.test(path)) return json(response, 200, { enabled: true });
      if (/^\/api\/(?:channels\/[^/]+\/)?media\//.test(path)) return reject(response, 503, 'TEST FIXTURE: no real media engine or SFU is connected.');
      const reaction = /^\/api\/chat\/channels\/([^/]+)\/messages\/([^/]+)\/reactions$/.exec(path);
      if (reaction && method === 'PUT') {
        const channel = channelFor(reaction[1]);
        if (!channel || !canParticipate(channel, user)) return reject(response, 404, 'resource not found');
        const who = state.chatSessions.get(request.headers['x-caper-chat-token']);
        if (!who || who.isGuest) return reject(response, 401, 'Messaging session required.');
        if (typeof body.emoji !== 'string' || !body.emoji || typeof body.active !== 'boolean') return reject(response, 400, 'Invalid fixture reaction.');
        const message = state.messages.get(channel.id)?.find(message => message.id === reaction[2]);
        if (!message) return reject(response, 404, 'Message not found.');
        return json(response, 200, react(channel.id, message, body.emoji, who.id, body.active));
      }
      if (reaction && method === 'GET') {
        // Who reacted: same read access as history, people in reaction order.
        const channel = channelFor(reaction[1]);
        if (!channel || !canRead(channel, user)) return reject(response, 404, 'resource not found');
        const message = state.messages.get(channel.id)?.find(message => message.id === reaction[2]);
        if (!message) return reject(response, 404, 'Message not found.');
        const person = id => (id === state.account.id ? state.account : accounts.find(account => account.id === id));
        return json(response, 200, {
          messageId: message.id, reactionSeq: message.reactionSeq ?? '0',
          reactions: (message.reactions ?? []).map(({ emoji, authorIds }) => ({ emoji, authors: authorIds.map(id => ({
            id, username: person(id)?.username ?? null, displayName: person(id)?.displayName ?? null, avatarId: person(id)?.avatarId ?? null,
          })) })),
        });
      }
      const chat = /^\/api\/chat\/channels\/([^/]+)\/messages$/.exec(path);
      if (path === '/api/chat/general' || chat) {
        const channel = channelFor(chat?.[1] ?? ids.demo);
        if (!channel) return reject(response, 404, 'Channel not found.');
        if (channel.id !== ids.demo && !user) return reject(response, 401, 'Sign in required.');
        if (!canRead(channel, user) || method === 'POST' && !canParticipate(channel, user)) return reject(response, 404, 'resource not found');
        const messages = state.messages.get(channel.id) ?? [];
        if (method === 'POST') {
          const who = state.chatSessions.get(request.headers['x-caper-chat-token']);
          if (!who) return reject(response, 401, 'Messaging session required.');
          if (typeof body.clientMessageId !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(body.clientMessageId)
            || typeof body.text !== 'string' || !body.text.trim() || [...body.text].length > 4000 || /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(body.text))
            return reject(response, 400, 'Enter a message of at most 4,000 characters.');
          // Rust parses UUIDs and serializes them lowercase; do not echo Swift's casing.
          const clientMessageId = body.clientMessageId.toLowerCase();
          const key = `${channel.id}:${clientMessageId}`;
          const previous = state.sendKeys.get(key);
          if (previous) return previous.token === request.headers['x-caper-chat-token'] && previous.text === body.text
            ? json(response, 200, previous.message) : reject(response, 409, 'Message ID already used.');
          const message = { id: randomUUID().replaceAll('-', '').slice(0, 15), channelId: channel.id, seq: String(BigInt(channelHead(channel.id)) + 1n), author: who,
            content: { version: 1, type: 'text', text: body.text }, createdAt: new Date().toISOString(), clientMessageId };
          messages.push(message); state.messages.set(channel.id, messages);
          state.sendKeys.set(key, { token: request.headers['x-caper-chat-token'], text: body.text, message });
          broadcast('chat', channel.id, { type: 'message.created', channelId: channel.id, seq: message.seq, message });
          return json(response, 200, message);
        }
        const before = url.searchParams.get('before');
        if (before !== null && !/^(0|[1-9]\d*)$/.test(before)) return reject(response, 400, 'invalid cursor');
        const available = before !== null ? messages.filter((message) => BigInt(message.seq) < BigInt(before)) : messages;
        return json(response, 200, { space: spaceFor(channel.spaceId), channel, messages: available.slice(-50), cursor: channelHead(channel.id), hasMore: available.length > 50 });
      }
      const spacePath = /^\/api\/spaces(?:\/([^/]+))?(?:\/channels\/([^/]+))?(?:\/(channels|members|invitations|invitation|membership)(?:\/([^/]+))?)?$/.exec(path);
      if (spacePath) {
        if (!user) return reject(response, 401, 'Sign in required.');
        const [, spaceId, channelId, section, memberId] = spacePath;
        if (!spaceId) {
          if (method === 'GET') return json(response, 200, {
            spaces: state.spaces.filter((detail) => detail.members.some((member) => member.id === user.id)).map((detail) => detail.space),
            invitations: state.spaces.filter((detail) => state.invitations.get(`${detail.space.id}:${user.id}`)?.status === 'pending').map((detail) => ({
              ...detail.space, inviter: { username: state.account.username, displayName: state.account.displayName },
            })), limits,
          });
          if (method === 'POST') {
            const name = typeof body.name === 'string' ? body.name.trim() : '';
            if (!name || [...name].length > 80 || /[\p{Cc}]/u.test(name)) return reject(response, 400, 'invalid space name');
            const id = randomUUID().replaceAll('-', '').slice(0, 12);
            const space = { id, name, ownerId: state.account.id };
            const channel = { id: randomUUID().replaceAll('-', '').slice(0, 12), spaceId: id, name: 'general', private: false };
            state.spaces.push({ space, channels: [channel], members: [{ ...state.account, owner: true }] }); state.messages.set(channel.id, []);
            state.joins.set(channel.id, [user.id]);
            return json(response, 201, space);
          }
        }
        const detail = state.spaces.find((entry) => entry.space.id === spaceId);
        if (!detail) return reject(response, 404, 'resource not found');
        const owner = detail.space.ownerId === user.id;
        const invitationKey = `${spaceId}:${user.id}`;
        if (section === 'invitation' && !channelId) {
          const invitation = state.invitations.get(invitationKey);
          if (invitation?.status !== 'pending') return reject(response, 404, 'resource not found');
          if (method === 'POST') {
            invitation.status = 'accepted'; detail.members.push(clone(user));
            const starter = detail.channels.find(channel => !channel.private && channel.name === 'general') ?? detail.channels.find(channel => !channel.private);
            if (starter) state.joins.set(starter.id, [...new Set([...(state.joins.get(starter.id) ?? []), user.id])]);
            return json(response, 200, detail.space);
          }
          if (method === 'DELETE') { invitation.status = 'declined'; return json(response, 204); }
        }
        if (!detail.members.some((member) => member.id === user.id)) return reject(response, 404, 'resource not found');
        if (section === 'invitations') {
          if (!owner) return reject(response, 404, 'resource not found');
          if (method === 'GET') return json(response, 200, { members: accounts.filter((member) => state.invitations.get(`${spaceId}:${member.id}`)?.status === 'pending') });
          const invitation = state.invitations.get(`${spaceId}:${memberId}`);
          if (method === 'DELETE' && invitation?.status === 'pending') { invitation.status = 'revoked'; return json(response, 204); }
          return reject(response, 404, 'resource not found');
        }
        const channel = detail.channels.find((entry) => entry.id === channelId);
        if (channelId && !channel) return reject(response, 404, 'Channel not found.');
        if (section === 'membership') {
          if (!canRead(channel, user)) return reject(response, 404, 'resource not found');
          const remaining = (state.joins.get(channel.id) ?? []).filter(id => id !== user.id);
          state.joins.set(channel.id, method === 'POST' ? [...remaining, user.id] : remaining);
          if (method === 'DELETE' && channel.private && !owner) {
            state.grants.set(channel.id, (state.grants.get(channel.id) ?? []).filter(id => id !== user.id));
            state.channelInvitations.set(`${channel.id}:${user.id}`, { status: 'revoked' });
          }
          return json(response, method === 'POST' ? 200 : 204, channelDTO(channel, user));
        }
        if (section === 'invitation' && channel) {
          const invitation = state.channelInvitations.get(`${channel.id}:${user.id}`);
          if (invitation?.status !== 'pending') return reject(response, 404, 'resource not found');
          invitation.status = method === 'POST' ? 'accepted' : 'declined';
          if (method === 'POST') {
            state.grants.set(channel.id, [...new Set([...(state.grants.get(channel.id) ?? []), user.id])]);
            state.joins.set(channel.id, [...new Set([...(state.joins.get(channel.id) ?? []), user.id])]);
          }
          return json(response, method === 'POST' ? 200 : 204, channelDTO(channel, user));
        }
        if (section === 'members') {
          if (channel && !owner) return reject(response, 404, 'resource not found');
          const list = channelId ? detail.members.filter((member) => (state.grants.get(channelId) ?? []).includes(member.id)) : detail.members;
          if (method === 'GET') return json(response, 200, { members: list, ...(channel ? { invitations: detail.members.filter(member => state.channelInvitations.get(`${channel.id}:${member.id}`)?.status === 'pending') } : {}) });
          if (method === 'POST') {
            if (!owner) return reject(response, 404, 'resource not found');
            const username = typeof body.username === 'string' ? body.username.trim().toLowerCase() : '';
            if (!/^[a-z0-9_]{3,32}$/.test(username)) return reject(response, 400, 'invalid username');
            const member = accounts.find((entry) => entry.username === username);
            if (!member) return reject(response, 404, 'user not found');
            if (channelId && !detail.members.some((entry) => entry.id === member.id)) return reject(response, 404, 'Member not found.');
            const exists = channelId ? (state.grants.get(channelId) ?? []).includes(member.id) : detail.members.some((entry) => entry.id === member.id);
            if (channelId) {
              if (!channel.private) return reject(response, 409, 'public channels are self-joined');
              const key = `${channelId}:${member.id}`;
              if (exists || member.id === user.id) return reject(response, 409, 'user already in channel');
              if (state.channelInvitations.get(key)?.status === 'pending') return reject(response, 409, 'user already invited');
              if (state.channelInvitations.has(key)) return reject(response, 409, 'invitation cooldown; try again after 24 hours');
              state.channelInvitations.set(key, { status: 'pending' });
            }
            else {
              if (exists) return reject(response, 409, 'user already in space');
              const key = `${spaceId}:${member.id}`;
              if (state.invitations.get(key)?.status === 'pending') return reject(response, 409, 'user already invited');
              if (state.invitations.has(key)) return reject(response, 409, 'invitation cooldown; try again after 24 hours');
              state.invitations.set(key, { status: 'pending' });
            }
            return json(response, exists ? 200 : 201, member);
          }
          if (method === 'DELETE') {
            if (!owner && (channelId || memberId !== user.id)) return reject(response, 404, 'resource not found');
            if (memberId === state.account.id) return reject(response, 409, 'The owner cannot be removed.');
            const exists = channelId ? (state.grants.get(channelId) ?? []).includes(memberId) : detail.members.some((member) => member.id === memberId);
            if (!exists && state.channelInvitations.get(`${channelId}:${memberId}`)?.status !== 'pending') return reject(response, 404, 'Member not found.');
            if (channelId) {
              state.grants.set(channelId, (state.grants.get(channelId) ?? []).filter((id) => id !== memberId));
              state.joins.set(channelId, (state.joins.get(channelId) ?? []).filter(id => id !== memberId));
              state.channelInvitations.set(`${channelId}:${memberId}`, { status: 'revoked' });
            }
            else {
              detail.members = detail.members.filter((member) => member.id !== memberId);
              state.invitations.set(`${spaceId}:${memberId}`, { status: 'revoked' });
              for (const item of detail.channels) state.grants.set(item.id, (state.grants.get(item.id) ?? []).filter((id) => id !== memberId));
              for (const item of detail.channels) {
                state.joins.set(item.id, (state.joins.get(item.id) ?? []).filter(id => id !== memberId));
                state.channelInvitations.set(`${item.id}:${memberId}`, { status: 'revoked' });
              }
            }
            return json(response, 204);
          }
        }
        if (section === 'channels' && method === 'POST') {
          if (!owner) return reject(response, 404, 'resource not found');
          if (!/^[a-z]+(?:-[a-z]+)*$/.test(body.name ?? '') || body.name.length > 80) return reject(response, 400, 'Use lowercase letters separated by single dashes.');
          if (detail.channels.some((entry) => entry.name === body.name)) return reject(response, 409, 'channel name already exists');
          const next = { id: randomUUID().replaceAll('-', '').slice(0, 12), spaceId, name: body.name, private: !!body.private };
          detail.channels.push(next); state.messages.set(next.id, []); state.grants.set(next.id, [state.account.id]);
          state.joins.set(next.id, [user.id]);
          return json(response, 201, channelDTO(next, user));
        }
        if (method === 'PATCH') {
          if (!owner) return reject(response, 404, 'resource not found');
          if (channel) {
            if (!/^[a-z]+(?:-[a-z]+)*$/.test(body.name ?? '') || body.name.length > 80) return reject(response, 400, 'invalid channel name');
            if (detail.channels.some((entry) => entry !== channel && entry.name === body.name)) return reject(response, 409, 'channel name already exists');
            Object.assign(channel, { name: body.name, private: !!body.private });
          } else {
            const name = typeof body.name === 'string' ? body.name.trim() : '';
            if (!name || [...name].length > 80 || /[\p{Cc}]/u.test(name)) return reject(response, 400, 'invalid space name');
            detail.space.name = name;
          }
          return json(response, 200, channel ? channelDTO(channel, user) : detail.space);
        }
        if (method === 'DELETE') {
          if (!owner) return reject(response, 404, 'resource not found');
          if (channel) detail.channels = detail.channels.filter((entry) => entry.id !== channelId);
          else state.spaces = state.spaces.filter((entry) => entry.space.id !== spaceId);
          return json(response, 204);
        }
        if (method === 'GET') return json(response, 200, channel ? channelDTO(channel, user) : {
          ...detail, channels: detail.channels.filter(channel => canRead(channel, user)).map(channel => channelDTO(channel, user)),
          channelInvitations: detail.channels.filter(channel => state.channelInvitations.get(`${channel.id}:${user.id}`)?.status === 'pending').map(channel => ({
            channel: { ...channel, joined: false }, inviter: { username: state.account.username, displayName: state.account.displayName },
          })),
        });
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
          if (!canRead(channelFor(channelId), identity(request))) return client.send({ type: 'error', id: frame.id, status: 404, error: 'resource not found' });
          if (!messages || !/^(0|[1-9]\d*)$/.test(frame.after ?? '0')) return client.send({ type: 'error', id: frame.id, status: 400, error: 'invalid subscription' });
          client.subscriptions.set(frame.id, frame);
          for (const event of channelEvents(channelId).filter(event => BigInt(event.seq) > BigInt(frame.after ?? '0')))
            client.send({ type: 'event', id: frame.id, event });
          client.send({ type: 'event', id: frame.id, event: { type: 'ready', cursor: channelHead(channelId) } });
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
          const channelId = frame.channelId ?? ids.demo;
          if (!canParticipate(channelFor(channelId), identity(request)) || state.mediaDenied.has(channelId))
            return client.send({ type: 'error', id: frame.id, status: 403, error: 'TEST FIXTURE: voice access denied.' });
          if (frame.token) return client.send({ type: 'error', id: frame.id, status: 503, error: 'TEST FIXTURE: no real media engine or SFU is connected.' });
          client.subscriptions.set(frame.id, frame);
          client.send({ type: 'event', id: frame.id, event: state.media.get(channelId) ?? { type: 'snapshot', revision: 0, participants: [] } });
        } else return client.send({ type: 'error', id: frame.id, status: 400, error: 'invalid subscription' });
        client.send({ type: 'subscribed', id: frame.id });
      } else if (frame.type === 'unsubscribe') client.subscriptions.delete(frame.id);
      else if (frame.type === 'command') {
        const id = frame.id.toLowerCase();
        if (frame.method === 'typing') {
          const who = state.chatSessions.get(frame.chatToken);
          const channel = channelFor(frame.channelId);
          if (!canParticipate(channel, identity(request))) return client.send({ type: 'result', id, status: 404, body: { error: 'resource not found' } });
          if (!who || !channel || typeof frame.body?.typing !== 'boolean') return client.send({ type: 'result', id, status: 401, body: { error: 'guest session expired' } });
          broadcast('chat', channel.id, { type: 'typing.updated', channelId: channel.id, author: who,
            typing: frame.body.typing, revision: String(++state.typingRevision) });
          client.send({ type: 'result', id, status: 204, body: {} });
        } else client.send({ type: 'result', id, status: 503, body: { error: 'TEST FIXTURE: no live SFU.' } });
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
