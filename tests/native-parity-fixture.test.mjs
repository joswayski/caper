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
      headers: { ...(auth ? { authorization: 'Bearer fixture-owner-token' } : {}),
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
  assert.deepEqual(saved.value, { id: ids.owner, username: 'new_owner', displayName: 'New Name' });
  assert.deepEqual((await request('/api/account/me', { auth: true })).value, saved.value);
  const detail = await request(`/api/spaces/${ids.space}`, { auth: true });
  assert.equal(detail.value.members.find(({ id }) => id === ids.owner).displayName, 'New Name');
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

test('space, channel, and member CRUD enforce production-shaped validation and idempotency', async (t) => {
  const { request } = await setup(t);
  assert.equal((await request('/api/spaces', { method: 'POST', auth: true, body: { name: 'bad\nname' } })).response.status, 400);
  const created = await request('/api/spaces', { method: 'POST', auth: true, body: { name: '  Field Notes  ' } });
  assert.equal(created.response.status, 201); assert.equal(created.value.name, 'Field Notes');
  const spaceId = created.value.id;
  const added = await request(`/api/spaces/${spaceId}/members`, { method: 'POST', auth: true, body: { username: 'maya' } });
  assert.equal(added.response.status, 201);
  assert.equal((await request(`/api/spaces/${spaceId}/members`, { method: 'POST', auth: true, body: { username: 'maya' } })).response.status, 200);
  const channel = await request(`/api/spaces/${spaceId}/channels`, { method: 'POST', auth: true, body: { name: 'private-notes', private: true } });
  assert.equal(channel.response.status, 201);
  assert.equal((await request(`/api/spaces/${spaceId}/channels`, { method: 'POST', auth: true, body: { name: 'private-notes', private: false } })).response.status, 409);
  assert.equal((await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, { method: 'POST', auth: true, body: { username: 'alex' } })).response.status, 404);
  assert.equal((await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, { method: 'POST', auth: true, body: { username: 'maya' } })).response.status, 201);
  assert.equal((await request(`/api/spaces/${spaceId}/members/${ids.member}`, { method: 'DELETE', auth: true })).response.status, 204);
  assert.equal((await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, { auth: true })).value.members.some(({ id }) => id === ids.member), false);
});

test('send is idempotent, conflicts on changed payload, and history uses stable head cursor pagination', async (t) => {
  const { request } = await setup(t);
  const session = await request('/api/chat/session', { method: 'POST', body: { name: 'Guest' } });
  const send = (clientMessageId, text) => request(`/api/chat/channels/${ids.demo}/messages`, {
    method: 'POST', headers: { 'x-caper-chat-token': session.value.token }, body: { clientMessageId, text },
  });
  const id = randomUUID();
  const first = await send(id, '  preserved  ');
  assert.equal(first.response.status, 200); assert.equal(first.value.content.text, '  preserved  ');
  assert.equal((await send(id, '  preserved  ')).value.id, first.value.id);
  assert.equal((await send(id, 'changed')).response.status, 409);
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
    headers: { 'x-caper-chat-token': session.value.token }, body: { clientMessageId: randomUUID(), text: 'live' } });
  assert.equal((await stream.next()).event.message.id, sent.value.id);
  stream.ws.send(JSON.stringify({ type: 'command', id: randomUUID(), issuedAt: Date.now(), method: 'typing', channelId: ids.demo,
    chatToken: session.value.token, body: { typing: true } }));
  assert.equal((await stream.next()).event.type, 'typing.updated');
  assert.equal((await stream.next()).status, 204);

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
