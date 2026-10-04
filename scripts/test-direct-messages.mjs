// Uses only the explicitly labelled, loopback-only native parity fixture.
// Start native-parity-fixture.mjs and Vite first. Never target a real account/API.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const web = process.env.DM_TEST_WEB_URL ?? 'http://127.0.0.1:5174';
const api = process.env.DM_TEST_API_URL ?? 'http://127.0.0.1:3001';
for (const url of [web, api]) assert.ok(['localhost', '127.0.0.1'].includes(new URL(url).hostname), 'Use a disposable loopback fixture');
const artifacts = process.env.DM_TEST_ARTIFACTS && resolve(process.env.DM_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const args = ['--session', 'direct-message-test'];
function browser(...command) {
  const result = JSON.parse(execFileSync('agent-browser', [...args, ...command, '--json'], { encoding: 'utf8', timeout: 60_000 }));
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = source => browser('eval', source).result;
const wait = expression => browser('wait', '--fn', expression);
const screenshot = name => {
  if (!artifacts) return;
  evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
  browser('screenshot', `${artifacts}/${name}.png`);
};
async function control(body) {
  const response = await fetch(`${api}/__fixture/control`, { method: 'POST', headers: { 'content-type': 'application/json', connection: 'close' }, body: JSON.stringify(body) });
  assert.equal(response.status, 200);
}
async function conversations() {
  return (await fetch(`${api}/api/dms`, { headers: { authorization: 'Bearer fixture-owner-token', connection: 'close' } })).json();
}
try {
  assert.equal((await (await fetch(`${api}/health`, { headers: { connection: 'close' } })).json()).fixture, true);
  await control({ reset: true });
  browser('open', 'about:blank');
  browser('set', 'viewport', '1440', '900', '2');
  browser('cookies', 'set', 'caper_fixture', 'owner', '--url', web, '--path', '/', '--sameSite', 'Lax');
  browser('open', `${web}/spaces`);
  wait('!!document.querySelector(".direct-section") && !!document.querySelector(".chat-composer textarea")');
  assert.equal(evaluate('document.querySelector(".direct-self small").textContent'), 'you');
  assert.equal(evaluate('document.querySelector(".direct-section").textContent.includes("No direct messages yet")'), false);
  assert.equal((await conversations()).conversations.length, 0, 'Showing the self row must not create a conversation');
  const plusOpacity = () => evaluate('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity');
  if (evaluate('matchMedia("(hover: hover)").matches')) {
    browser('mouse', 'move', '700', '400');
    wait('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity === "0"');
    browser('hover', '.direct-section-title');
    wait('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity === "1"');
    browser('click', '.direct-section [aria-label="New direct message"]');
    wait('!!document.querySelector(".space-dialog[open]")');
    browser('click', '[aria-label="Close New direct message"]');
    browser('mouse', 'move', '700', '400');
    evaluate(`(() => { const buttons = [...document.querySelectorAll('button:not(:disabled), a[href], summary, textarea:not(:disabled)')].filter(node => node.getClientRects().length);
      const plus = document.querySelector('.direct-section [aria-label="New direct message"]');
      buttons[buttons.indexOf(plus) - 1].focus(); })()`);
    browser('press', 'Tab');
    assert.equal(evaluate('document.activeElement?.getAttribute("aria-label")'), 'New direct message', 'Hidden plus stays in the keyboard focus order');
    wait('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity === "1"');
    browser('press', 'Enter');
    wait('!!document.querySelector(".space-dialog[open]")');
    browser('click', '[aria-label="Close New direct message"]');
    evaluate('document.activeElement.blur()');
    wait('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity === "0"');
    assert.equal(plusOpacity(), '0');
  } else {
    assert.equal(plusOpacity(), '1', 'Non-hover clients keep the DM action visible');
  }
  assert.ok(evaluate(`(() => { const panel = document.querySelector('.people-panel').getBoundingClientRect();
    return ['.channel-navigation > header', '.direct-section'].every(selector => {
      const node = document.querySelector(selector), r = node.getBoundingClientRect();
      const s = getComputedStyle(node, selector.includes('header') ? '::after' : '::before');
      return Math.abs(r.left + parseFloat(s.left) - panel.left) < 1 && Math.abs(r.right - parseFloat(s.right) - (panel.right - 1)) < 1;
    }); })()`), 'Sidebar dividers must span the panel, not just its padded contents');
  screenshot('dm-empty-list');
  browser('click', '.direct-action');
  wait('document.querySelector(".space-dialog h2")?.textContent === "Manage space"');
  browser('fill', '.member-add input', 'sam');
  browser('click', '.member-add button[type="submit"]');
  wait('document.querySelector(".member-manager")?.textContent.includes("@sam · Invited")');
  screenshot('dm-invite-people');
  browser('click', '[aria-label="Close Manage space"]');
  browser('click', '[aria-label="New direct message"]');
  wait('!!document.querySelector(".space-dialog[open]")');
  browser('fill', '.space-dialog input', 'missing_account');
  browser('click', '.space-dialog button[type="submit"]');
  wait('!!document.querySelector(".space-form-error")');
  assert.match(evaluate('document.querySelector(".space-form-error").textContent'), /not found/i);
  screenshot('dm-start-error');
  browser('fill', '.space-dialog input', 'fixture_alex');
  browser('click', '.space-dialog button[type="submit"]');
  wait('document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex") && !document.querySelector(".space-dialog[open]")');
  assert.equal(evaluate('document.querySelector(".space-member-presence")'), null);
  assert.equal(evaluate('document.querySelectorAll("#space-channel-list > li").length'), 3, 'DM selection retains the space channels');
  assert.equal(evaluate('document.querySelector(".chat-heading").textContent.includes("#")'), false);
  browser('fill', '.chat-composer textarea', 'TEST FIXTURE — a private conversation across spaces.');
  browser('press', 'Enter');
  wait('document.querySelector(".chat-messages")?.textContent.includes("a private conversation across spaces.")');
  await control({ incomingMessage: { channelId: 'dm0000000001', text: 'A live reply from Alex.' } });
  wait('document.querySelector(".chat-messages")?.textContent.includes("A live reply from Alex.")');
  wait('!document.querySelector(".direct-unread")');
  screenshot('dm-desktop');
  browser('click', '#space-channel-list li:first-child .channel-select');
  wait('document.querySelector(".chat-heading")?.textContent.includes("general")');
  assert.equal(evaluate('document.querySelector(".chat-messages").textContent.includes("A live reply from Alex.")'), false, 'DM history cannot bleed into a channel');
  await control({ incomingMessage: { channelId: 'dm0000000001', text: 'Unread while browsing a channel.' } });
  browser('reload');
  wait('!!document.querySelector(".direct-unread")');
  browser('click', '.direct-select:not(.direct-self)');
  wait('document.querySelector(".chat-messages")?.textContent.includes("Unread while browsing a channel.") && !document.querySelector(".direct-unread")');
  assert.equal((await conversations()).conversations[0].readSeq, '3');
  const dmHistory = await (await fetch(`${api}/api/chat/channels/dm0000000001/messages`, { headers: { authorization: 'Bearer fixture-owner-token', connection: 'close' } })).json();
  const reactionMessage = dmHistory.messages[0].id;
  await control({ incomingReaction: { channelId: 'dm0000000001', messageId: reactionMessage, emoji: '🎉' } });
  wait('document.querySelector(".chat-reaction")?.getAttribute("aria-label") === "🎉, 1 reaction"');
  wait('fetch("/api/dms").then(r => r.json()).then(v => v.conversations[0].readSeq === "4")');
  assert.equal((await conversations()).conversations[0].readSeq, '4', 'Reactions advance read receipts even though the newest message is still sequence 3');
  browser('click', '.chat-reaction');
  wait('document.querySelector(".chat-reaction")?.getAttribute("aria-pressed") === "true" && !document.querySelector(".chat-reaction").disabled');
  assert.equal(evaluate('document.querySelector(".chat-reaction").textContent'), '2');
  screenshot('dm-reactions-desktop');
  browser('click', '.chat-reaction');
  wait('document.querySelector(".chat-reaction")?.getAttribute("aria-pressed") === "false" && !document.querySelector(".chat-reaction").disabled');
  assert.equal(evaluate('document.querySelector(".chat-reaction").textContent'), '1');
  browser('reload');
  wait('document.querySelector(".chat-reaction")?.getAttribute("aria-label") === "🎉, 1 reaction"');
  wait('fetch("/api/dms").then(r => r.json()).then(v => v.conversations[0].readSeq === "6")');
  // Reopening the canonical pair must not create a second conversation.
  browser('click', '[aria-label="New direct message"]');
  browser('fill', '.space-dialog input', '@fixture_alex');
  browser('click', '.space-dialog button[type="submit"]');
  wait('!document.querySelector(".space-dialog[open]")');
  assert.equal((await conversations()).conversations.length, 1);
  browser('click', '[aria-label="Create space"]');
  browser('fill', '.space-dialog input', 'TEST FIXTURE Second space');
  browser('click', '.space-dialog button[type="submit"]');
  wait('document.querySelector(".space-menu h1")?.textContent === "TEST FIXTURE Second space" && document.querySelector(".chat-heading")?.textContent.includes("general")');
  assert.equal(evaluate('document.querySelectorAll("#space-channel-list > li").length'), 1);
  assert.equal(evaluate('document.querySelectorAll(".direct-select").length'), 2, 'Self row and the same global DM in a different space');
  browser('click', '.direct-select:not(.direct-self)');
  wait('document.querySelector(".chat-messages")?.textContent.includes("A live reply from Alex.")');
  assert.equal(evaluate('document.querySelector("vite-error-overlay")'), null, 'Switching channel/DM/space must not surface cancelled requests as dev errors');
  screenshot('dm-second-space');
  browser('set', 'viewport', '390', '844', '2');
  wait('!!document.querySelector(".navigation-toggle")');
  assert.ok(evaluate('document.documentElement.scrollWidth <= innerWidth'), 'Narrow conversation must not overflow');
  assert.equal(evaluate('getComputedStyle(document.querySelector(".sidebar-channels")).display'), 'none', 'Narrow DM navigation must not crowd the conversation');
  screenshot('dm-narrow');
  browser('click', '.navigation-toggle');
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  screenshot('dm-narrow-browse');
  browser('click', '.direct-select:not(.direct-self)');
  wait('!document.querySelector(".spaces-room.navigation-open")');
  browser('fill', '.chat-composer textarea', 'A global draft survives losing a space.');
  await control({ noSpaces: true });
  evaluate('window.dispatchEvent(new Event("focus"))');
  wait('!new URL(location.href).searchParams.has("space") && new URL(location.href).searchParams.get("dm") === "dm0000000001" && document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex")');
  assert.equal(evaluate('document.querySelector(".chat-composer textarea").value'), 'A global draft survives losing a space.');
  assert.ok(evaluate('document.querySelector(".chat-messages").textContent.includes("A live reply from Alex.")'), 'Space revocation must not clear global DM history');
  browser('open', `${web}/spaces?dm=dm0000000001`);
  wait('document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex")');
  screenshot('dm-no-spaces');
  assert.ok(evaluate('document.querySelector(".chat-messages").textContent.includes("A live reply from Alex.")'), 'Global DM survives loss of space membership');
  assert.equal(evaluate('document.querySelector(".chat-composer textarea").disabled'), false, 'No-space DMs remain writable');
  browser('fill', '.chat-composer textarea', 'TEST FIXTURE — still messaging without a space.');
  browser('press', 'Enter');
  wait('document.querySelector(".chat-messages")?.textContent.includes("still messaging without a space.")');
  browser('open', `${web}/spaces`);
  wait('!!document.querySelector(".spaces-empty .direct-select")');
  assert.equal(evaluate('document.querySelector(".spaces-empty h1").textContent'), 'Name your space', 'Global DMs must preserve first-space onboarding');
  assert.ok(evaluate('document.documentElement.scrollWidth <= innerWidth'));
  screenshot('dm-first-space');
  browser('click', '[aria-label="New direct message"]');
  browser('fill', '.space-dialog input', 'fixture_alex');
  browser('click', '.space-dialog button[type="submit"]');
  wait('document.querySelector(".chat-messages")?.textContent.includes("still messaging without a space.") && !document.querySelector(".space-dialog[open]")');
  assert.equal((await conversations()).conversations.length, 1, 'No-space onboarding can reopen the existing global DM');
  browser('click', '.navigation-toggle');
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  assert.equal(evaluate('document.querySelector(".direct-action").textContent'), 'New message', 'No-space accounts cannot invite people to a space');
  await control({ failure: { path: '/api/dms', method: 'POST', status: 503, error: 'TEST FIXTURE: notes temporarily unavailable' } });
  browser('click', '.direct-self');
  wait('document.querySelector(".direct-section [role=alert]")?.textContent.includes("notes temporarily unavailable")');
  screenshot('dm-self-error');
  assert.equal((await conversations()).conversations.length, 1);
  // Hold the creation request while the user chooses another conversation.
  // A late response may add the notes row, but must not steal that selection.
  evaluate(`(() => {
    const fetch = window.fetch.bind(window);
    window.fetch = async (...args) => {
      if (args[0] === '/api/dms' && args[1]?.method === 'POST') {
        window.fetch = fetch;
        await new Promise(resolve => { window.releaseNotesRequest = resolve; });
      }
      return fetch(...args);
    };
  })()`);
  browser('click', '.direct-self');
  wait('typeof window.releaseNotesRequest === "function" && document.querySelector(".direct-self").disabled');
  browser('click', '.direct-select:not(.direct-self)');
  wait('document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex") && !document.querySelector(".spaces-room.navigation-open")');
  evaluate('window.releaseNotesRequest()');
  wait('!document.querySelector(".direct-self").disabled');
  assert.equal(new URL(evaluate('location.href')).searchParams.get('dm'), 'dm0000000001', 'Late notes creation must not override a subsequent selection');
  assert.ok(evaluate('document.querySelector(".chat-heading").textContent.includes("TEST FIXTURE Alex")'));
  browser('click', '.navigation-toggle');
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  browser('click', '.direct-self');
  wait('document.querySelector(".chat-heading")?.textContent.includes("Fixture Owner") && !document.querySelector(".spaces-room.navigation-open")');
  browser('fill', '.chat-composer textarea', 'TEST FIXTURE — private notes that survive a reload.');
  browser('press', 'Enter');
  wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
  browser('reload');
  wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
  browser('set', 'viewport', '1440', '900', '2');
  wait('!!document.querySelector(".direct-self[aria-current=page]")');
  assert.equal(evaluate('document.querySelectorAll(".direct-self").length'), 1, 'Stored notes must not duplicate the pinned self row');
  screenshot('dm-self-desktop');
  browser('click', '.direct-select:not(.direct-self)');
  wait('document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex")');
  assert.equal(evaluate('document.querySelector(".chat-messages").textContent.includes("private notes that survive a reload.")'), false);
  browser('click', '.direct-self');
  wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
  const saved = (await conversations()).conversations;
  assert.equal(saved.length, 2, 'Reopening self notes must reuse the same conversation');
  assert.equal(saved.find(item => item.peer.id === 'owner0000001').id, 'dm0000000002');
  browser('set', 'viewport', '390', '844', '2');
  browser('click', '.navigation-toggle');
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  assert.ok(evaluate('document.documentElement.scrollWidth <= innerWidth'));
  screenshot('dm-self-narrow-browse');
  console.log('PASS: self notes lazy creation/error/reuse/send/persistence/isolation/navigation race, full-width dividers, real space invitation action, DM start/error/reopen/live replies/reactions/read receipts, space independence, and desktop/narrow navigation (disposable fixture).');
} finally {
  browser('close');
}
