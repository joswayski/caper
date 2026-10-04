import test from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { startFixture, fixtureIDs as ids } from '../scripts/native-parity-fixture.mjs';

async function setup(t) {
  const fixture = await startFixture({ port: 0, gatewayPort: 0 });
  t.after(() => fixture.close());
  const base = `http://127.0.0.1:${fixture.port}`;
  const request = async (path, { auth = false, body, ...init } = {}) => {
    const response = await fetch(base + path, {
      ...init,
      headers: { ...(auth ? { authorization: `Bearer ${auth === true ? 'fixture-owner-token' : auth}` } : {}),
        ...(body === undefined ? {} : { 'content-type': 'application/json' }), ...init.headers },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const value = response.status === 204 ? undefined : await response.json();
    return { response, value };
  };
  return { fixture, base, request };
}

function socket(url) {
  const ws = new WebSocket(url);
  const queued = [];
  const waiting = [];
  ws.addEventListener('message', ({ data }) => {
    const value = JSON.parse(data);
    const resolve = waiting.shift();
    if (resolve) resolve(value); else queued.push(value);
  });
  const next = () => queued.length ? Promise.resolve(queued.shift()) : new Promise((resolve) => waiting.push(resolve));
  return { ws, next, opened: new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve, { once: true }); ws.addEventListener('error', reject, { once: true });
  }) };
}

test('email code auth and validated profile persist across account and member reads', async (t) => {
  const { request } = await setup(t);
  assert.equal((await request('/api/account/me')).response.status, 401);
  assert.equal((await request('/api/auth/email/request', { method: 'POST', body: { email: 'bad' } })).response.status, 400);
  const challenge = await request('/api/auth/email/request', { method: 'POST', body: { email: 'owner@example.test' } });
  const wrong = await request('/api/auth/email/verify', { method: 'POST', body: { challengeId: challenge.value.challengeId, code: 'WRONG' } });
  assert.equal(wrong.response.status, 401); assert.equal(wrong.value.attemptsRemaining, 2);
  const verified = await request('/api/auth/email/verify', { method: 'POST', body: { challengeId: challenge.value.challengeId, code: 'ABC234' } });
  assert.equal(verified.value.account.id, ids.owner);
  assert.equal((await request('/api/account/profile', { method: 'POST', auth: true, body: { username: 'no-dashes', displayName: 'Owner' } })).response.status, 400);
  const saved = await request('/api/account/profile', { method: 'POST', auth: true, body: { username: ' New_Owner ', displayName: ' New Name ' } });
  assert.deepEqual(saved.value, { id: ids.owner, username: 'new_owner', displayName: 'New Name', avatarId: 0 });
  assert.deepEqual((await request('/api/account/me', { auth: true })).value, saved.value);
  const detail = await request(`/api/spaces/${ids.space}`, { auth: true });
  assert.equal(detail.value.members.find(({ id }) => id === ids.owner).displayName, 'New Name');
  assert.deepEqual(detail.value.members.map(member => member.avatarId), [0, 31, 799]);
  const history = await request(`/api/chat/channels/${ids.general}/messages`, { auth: true });
  assert.deepEqual(history.value.messages.map(message => message.author.avatarId), [0, 31, 31, 799]);
});

test('persistent navigation failure survives prefetch until explicitly cleared', async (t) => {
  const { request } = await setup(t);
  const path = `/api/spaces/${ids.space}`;
  await request('/__fixture/control', { method: 'POST', body: { failure: { path, method: 'GET', status: 503, persistent: true } } });
  assert.equal((await request(path, { auth: true })).response.status, 503, 'hover read fails');
  assert.equal((await request(path, { auth: true })).response.status, 503, 'click read still fails');
  await request('/__fixture/control', { method: 'POST', body: { clearFailures: true } });
  assert.equal((await request(path, { auth: true })).response.status, 200, 'explicit retry can succeed');
  await request('/__fixture/control', { method: 'POST', body: { failure: { path, status: 502 } } });
  assert.equal((await request(path, { auth: true })).response.status, 502);
  assert.equal((await request(path, { auth: true })).response.status, 200, 'ordinary failures remain one-shot');
});

test('space invitation consent and channel grants enforce distinct access contracts', async (t) => {
  const { request } = await setup(t);
  assert.equal((await request('/api/spaces', { method: 'POST', auth: true, body: { name: 'bad\nname' } })).response.status, 400);
  const created = await request('/api/spaces', { method: 'POST', auth: true, body: { name: '  Field Notes  ' } });
  assert.equal(created.response.status, 201); assert.equal(created.value.name, 'Field Notes');
  const spaceId = created.value.id;
  const added = await request(`/api/spaces/${spaceId}/members`, { method: 'POST', auth: true, body: { username: 'maya' } });
  assert.equal(added.response.status, 201);
  assert.equal((await request(`/api/spaces/${spaceId}/members`, { method: 'POST', auth: true, body: { username: 'maya' } })).response.status, 409);
  assert.equal((await request(`/api/spaces/${spaceId}/invitations`, { auth: true })).value.members[0].id, ids.member);
  assert.equal((await request(`/api/spaces/${spaceId}`, { auth: 'fixture-member-token' })).response.status, 404);
  const pending = await request('/api/spaces', { auth: 'fixture-member-token' });
  assert.deepEqual(pending.value.invitations.map(({ id }) => id), [spaceId]);
  assert.deepEqual(pending.value.invitations[0].inviter, { username: 'fixture_owner', displayName: 'Fixture Owner' });
  assert.ok(!pending.value.spaces.some(({ id }) => id === spaceId));
  const channel = await request(`/api/spaces/${spaceId}/channels`, { method: 'POST', auth: true, body: { name: 'private-notes', private: true } });
  assert.equal(channel.response.status, 201);
  assert.equal((await request(`/api/spaces/${spaceId}/channels`, { method: 'POST', auth: true, body: { name: 'private-notes', private: false } })).response.status, 409);
  assert.equal((await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, { method: 'POST', auth: true, body: { username: 'alex' } })).response.status, 404);
  assert.equal((await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, { method: 'POST', auth: true, body: { username: 'maya' } })).response.status, 404);
  assert.equal((await request(`/api/spaces/${spaceId}/invitation`, { method: 'POST', auth: 'fixture-member-token' })).response.status, 200);
  assert.equal((await request(`/api/spaces/${spaceId}/invitation`, { method: 'POST', auth: 'fixture-member-token' })).response.status, 404);
  assert.equal((await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, { method: 'POST', auth: true, body: { username: 'maya' } })).response.status, 201);
  assert.equal((await request(`/api/spaces/${spaceId}/members/${ids.member}`, { method: 'DELETE', auth: true })).response.status, 204);
  assert.equal((await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, { auth: true })).value.members.some(({ id }) => id === ids.member), false);
  assert.equal((await request(`/api/spaces/${spaceId}`, { auth: 'fixture-member-token' })).response.status, 404);
});

test('management can invite a nonmember without bypassing removed-member cooldown', async (t) => {
  const { request } = await setup(t);
  const root = `/api/spaces/${ids.space}`;
  assert.equal((await request(`${root}/members/${ids.member}`, { method: 'DELETE', auth: true })).response.status, 204);
  const reinvite = await request(`${root}/members`, { method: 'POST', auth: true, body: { username: 'maya' } });
  assert.equal(reinvite.response.status, 409);
  assert.match(reinvite.value.error, /cooldown/);
  assert.deepEqual((await request(`${root}/invitations`, { auth: true })).value.members, []);

  const invited = await request(`${root}/members`, { method: 'POST', auth: true, body: { username: 'sam' } });
  assert.equal(invited.response.status, 201);
  assert.equal(invited.value.id, ids.invitee);
  assert.deepEqual((await request(`${root}/invitations`, { auth: true })).value.members.map(member => member.username), ['sam']);
  assert.deepEqual((await request(root, { auth: true })).value.members.map(member => member.id), [ids.owner, ids.other],
    'a pending invitation must not grant membership');
  assert.equal((await request(`${root}/channels/${ids.private}/members`, {
    method: 'POST', auth: true, body: { username: 'sam' },
  })).response.status, 404, 'a pending invitee cannot receive private channel membership');
});

test('public preview stays readable without participation; private consent grants and joins together', async (t) => {
  const { request } = await setup(t);
  const publicRoot = `/api/spaces/${ids.space}/channels/${ids.design}`;
  const privateRoot = `/api/spaces/${ids.space}/channels/${ids.private}`;
  const member = { auth: 'fixture-member-token' };
  assert.equal((await request(`${publicRoot}/membership`, { ...member, method: 'DELETE' })).response.status, 204);
  assert.equal((await request(`/api/chat/channels/${ids.design}/messages`, member)).response.status, 200);
  assert.equal((await request(`/api/channels/${ids.design}/media/status`, member)).response.status, 404);
  assert.equal((await request(`/api/chat/channels/${ids.design}/messages`, { ...member, method: 'POST', body: { text: 'not joined' } })).response.status, 404);
  assert.equal((await request(`/api/spaces/${ids.space}`, member)).value.channels.find(channel => channel.id === ids.design).joined, false);
  assert.equal((await request(`${publicRoot}/membership`, { ...member, method: 'POST' })).value.joined, true);

  assert.equal((await request(`${privateRoot}/members/${ids.member}`, { auth: true, method: 'DELETE' })).response.status, 204);
  // Reset only the disposable fixture's invitation tombstone to exercise a new invitation.
  await request('/__fixture/control', { method: 'POST', body: { reset: true } });
  const created = await request(`/api/spaces/${ids.space}/channels`, { auth: true, method: 'POST', body: { name: 'private-consent', private: true } });
  const root = `/api/spaces/${ids.space}/channels/${created.value.id}`;
  assert.equal((await request(`${root}/members`, { auth: true, method: 'POST', body: { username: 'maya' } })).response.status, 201);
  const before = (await request(`/api/spaces/${ids.space}`, member)).value;
  assert.ok(!before.channels.some(channel => channel.id === created.value.id));
  assert.equal(before.channelInvitations[0].inviter.username, 'fixture_owner');
  assert.equal((await request(`/api/chat/channels/${created.value.id}/messages`, member)).response.status, 404);
  assert.equal((await request(`${root}/membership`, { ...member, method: 'POST' })).response.status, 404);
  assert.equal((await request(`${root}/invitation`, { ...member, method: 'POST' })).value.joined, true);
  assert.equal((await request(`/api/chat/channels/${created.value.id}/messages`, member)).response.status, 200);
  assert.equal((await request(`${root}/membership`, { ...member, method: 'DELETE' })).response.status, 204);
  assert.equal((await request(`/api/chat/channels/${created.value.id}/messages`, member)).response.status, 404);
});

test('send is idempotent, conflicts on changed payload, and history uses stable head cursor pagination', async (t) => {
  const { request } = await setup(t);
  const session = await request('/api/chat/session', { method: 'POST', body: { name: 'Guest' } });
  const send = (clientMessageId, text) => request(`/api/chat/channels/${ids.demo}/messages`, {
    method: 'POST', headers: { 'x-caper-chat-token': session.value.token }, body: { clientMessageId, text },
  });
  const id = 'AB12CD34-EF56-4789-8ABC-DEF012345678';
  const canonicalId = 'ab12cd34-ef56-4789-8abc-def012345678';
  const first = await send(id, '  preserved  ');
  assert.equal(first.response.status, 200); assert.equal(first.value.content.text, '  preserved  ');
  assert.equal(first.value.clientMessageId, canonicalId, 'match Rust UUID serialization instead of echoing Swift casing');
  assert.equal((await send(id, '  preserved  ')).value.id, first.value.id);
  assert.equal((await send(canonicalId, '  preserved  ')).value.id, first.value.id, 'UUID casing cannot create a second message');
  assert.equal((await send(canonicalId, 'changed')).response.status, 409);
  assert.equal((await send('not-a-uuid', 'text')).response.status, 400);
  for (let index = 0; index < 48; index++) await send(randomUUID(), `message ${index}`);
  const latest = await request(`/api/chat/channels/${ids.demo}/messages`);
  assert.equal(latest.value.messages.length, 50); assert.equal(latest.value.hasMore, true); assert.equal(latest.value.cursor, '53');
  const older = await request(`/api/chat/channels/${ids.demo}/messages?before=${latest.value.messages[0].seq}`);
  assert.equal(older.value.cursor, latest.value.cursor, 'older pages retain the committed channel head');
  assert.ok(older.value.messages.every((message) => BigInt(message.seq) < BigInt(latest.value.messages[0].seq)));
  assert.equal((await request(`/api/chat/channels/${ids.demo}/messages?before=-1`)).response.status, 400);
});

test('loopback WebSocket replays, delivers live messages/typing/presence, failures, and disconnects', async (t) => {
  const { fixture, base, request } = await setup(t);
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`);
  t.after(() => stream.ws.close());
  await stream.opened;
  assert.equal((await stream.next()).type, 'hello');
  stream.ws.send(JSON.stringify({ type: 'subscribe', id: 'chat', kind: 'chat', channelId: ids.demo, after: '3' }));
  const replay = await stream.next();
  assert.equal(replay.event.message.seq, '4');
  assert.deepEqual((await stream.next()).event, { type: 'ready', cursor: '4' });
  assert.equal((await stream.next()).type, 'subscribed');

  const session = await request('/api/chat/session', { method: 'POST', body: { name: 'Socket Guest' } });
  const sent = await request(`/api/chat/channels/${ids.demo}/messages`, { method: 'POST',
    headers: { 'x-caper-chat-token': session.value.token }, body: { clientMessageId: 'AB12CD34-EF56-4789-8ABC-DEF012345678', text: 'live' } });
  const liveMessage = (await stream.next()).event.message;
  assert.equal(liveMessage.id, sent.value.id);
  assert.equal(liveMessage.clientMessageId, 'ab12cd34-ef56-4789-8abc-def012345678');
  stream.ws.send(JSON.stringify({ type: 'command', id: 'BC23DE45-FA67-489A-9BCD-EF0123456789', issuedAt: Date.now(), method: 'typing', channelId: ids.demo,
    chatToken: session.value.token, body: { typing: true } }));
  assert.equal((await stream.next()).event.type, 'typing.updated');
  const result = await stream.next();
  assert.equal(result.status, 204);
  assert.equal(result.id, 'bc23de45-fa67-489a-9bcd-ef0123456789', 'command results also serialize a Rust UUID');

  stream.ws.send(JSON.stringify({ type: 'subscribe', id: 'presence', kind: 'presence', spaceId: ids.space, userIds: [ids.owner, ids.other] }));
  assert.deepEqual((await stream.next()).event.members.map(({ status }) => status), ['online', 'idle']);
  assert.equal((await stream.next()).type, 'subscribed');
  stream.ws.send(JSON.stringify({ type: 'command', id: randomUUID(), issuedAt: Date.now(), method: 'media.join', body: {} }));
  assert.equal((await stream.next()).status, 503);

  await request('/__fixture/control', { method: 'POST', body: { failure: { path: '/api/account/me', status: 418, error: 'planned' } } });
  assert.equal((await request('/api/account/me', { auth: true })).response.status, 418);
  const closed = new Promise((resolve) => stream.ws.addEventListener('close', resolve, { once: true }));
  await fetch(`${base}/__fixture/control`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ disconnect: true }) });
  await closed;
});

test('spectator rosters update and revoke without granting capture or account-channel access', async (t) => {
  const { fixture, request } = await setup(t);
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`);
  t.after(() => stream.ws.close());
  await stream.opened;
  assert.equal((await stream.next()).type, 'hello');
  stream.ws.send(JSON.stringify({ type: 'subscribe', id: 'private', kind: 'media', channelId: ids.private }));
  assert.equal((await stream.next()).status, 403);
  stream.ws.send(JSON.stringify({ type: 'subscribe', id: 'demo', kind: 'media' }));
  assert.deepEqual((await stream.next()).event, { type: 'snapshot', revision: 0, participants: [] });
  assert.equal((await stream.next()).type, 'subscribed');
  const participant = { id: 'guest-voice', name: 'TEST FIXTURE Guest', muted: true, deafened: false };
  await request('/__fixture/control', { method: 'POST', body: { media: { sessionStartedAt: 12345, participants: [{ ...participant, tracks: ['must not leak'] }] } } });
  assert.deepEqual(await stream.next(), { type: 'event', id: 'demo', event: { type: 'snapshot', revision: 1, sessionStartedAt: 12345, participants: [participant] } });
  await request('/__fixture/control', { method: 'POST', body: { media: { participants: [{ ...participant, muted: false }] } } });
  assert.equal((await stream.next()).event.sessionStartedAt, 12345, 'roster changes retain the shared session start');
  await request('/__fixture/control', { method: 'POST', body: { media: { participants: [] } } });
  assert.deepEqual((await stream.next()).event, { type: 'snapshot', revision: 3, sessionStartedAt: null, participants: [] });
  await request('/__fixture/control', { method: 'POST', body: { mediaAccessDenied: {} } });
  assert.equal((await stream.next()).status, 403);
  await request('/__fixture/control', { method: 'POST', body: { media: { sessionStartedAt: 67890, participants: [participant] } } });
  stream.ws.send(JSON.stringify({ type: 'heartbeat' }));
  assert.equal((await stream.next()).type, 'heartbeat', 'revocation removed the spectator subscription');
  stream.ws.send(JSON.stringify({ type: 'subscribe', id: 'retry', kind: 'media' }));
  assert.equal((await stream.next()).status, 403);
  await request('/__fixture/control', { method: 'POST', body: { mediaAccessDenied: { denied: false } } });
  stream.ws.send(JSON.stringify({ type: 'subscribe', id: 'restored', kind: 'media' }));
  assert.deepEqual(await stream.next(), { type: 'event', id: 'restored', event: { type: 'snapshot', revision: 4, sessionStartedAt: 67890, participants: [participant] } });
  assert.equal((await stream.next()).type, 'subscribed');
  assert.equal((await request('/api/media/join', { method: 'POST', body: {} })).response.status, 503);
});

test('DM reaction fixture advances stream and read heads without changing message sequences', async (t) => {
  const { request } = await setup(t);
  await request('/api/dms', { auth: true, method: 'POST', body: { username: 'fixture_alex' } });
  const session = await request('/api/chat/session', { auth: true, method: 'POST', body: { name: 'Fixture Owner' } });
  const root = `/api/chat/channels/${ids.direct}/messages`;
  const sent = await request(root, { auth: true, method: 'POST', headers: { 'x-caper-chat-token': session.value.token }, body: { clientMessageId: randomUUID(), text: 'TEST FIXTURE — reactions' } });
  await request('/__fixture/control', { method: 'POST', body: { incomingReaction: { channelId: ids.direct, messageId: sent.value.id, emoji: '🎉' } } });
  const reactionPath = `${root}/${sent.value.id}/reactions`;
  const mutation = { auth: true, method: 'PUT', headers: { 'x-caper-chat-token': session.value.token }, body: { emoji: '🎉', active: true } };
  const added = await request(reactionPath, mutation);
  assert.equal(added.value.seq, '3');
  assert.deepEqual(added.value.reactions, [{ emoji: '🎉', authorIds: [ids.other, ids.owner] }]);
  assert.deepEqual((await request(reactionPath, mutation)).value, added.value, 'no-op does not allocate a sequence');
  const history = (await request(root, { auth: true })).value;
  assert.equal(history.cursor, '3');
  assert.equal(history.messages[0].seq, '1');
  assert.equal(history.messages[0].reactionSeq, '3');
  await request(`/api/dms/${ids.direct}/read`, { auth: true, method: 'POST', body: { seq: '3' } });
  const dm = (await request('/api/dms', { auth: true })).value.conversations[0];
  assert.equal(dm.lastSeq, '3');
  assert.equal(dm.readSeq, '3');
  await request('/__fixture/control', { method: 'POST', body: { incomingMessage: { channelId: ids.direct, text: 'After reactions' } } });
  assert.equal((await request(root, { auth: true })).value.messages.at(-1).seq, '4');
});
