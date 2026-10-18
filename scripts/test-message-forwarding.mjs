// Disposable loopback fixture + Chromium. These checks are not native-device acceptance.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const web = process.env.MESSAGE_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.MESSAGE_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const browser = (...args) => {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", "forward-check", ...args, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (source) => browser("eval", source).result;
const wait = (source) => browser("wait", "--fn", source);
const screenshot = (name) => {
  if (!artifacts) return;
  evaluate("document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))");
  assert.equal(evaluate("devicePixelRatio"), 2);
  const target = evaluate('!!document.querySelector(".chat-forward-dialog")') ? [".chat-forward-dialog"] : [];
  browser("screenshot", ...target, `${artifacts}/${name}.png`);
};
const request = async (path, body, who = "owner", method = body ? "POST" : "GET", token) => {
  const response = await fetch(api + path, {
    method,
    headers: {
      authorization: `Bearer fixture-${who}-token`,
      connection: "close",
      ...(body ? { "content-type": "application/json" } : {}),
      ...(token ? { "x-caper-chat-token": token } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  return { status: response.status, value: await response.json() };
};
const control = (body) => request("/__fixture/control", body);
const checkReactionLayout = () => {
  assert.equal(
    evaluate(`(() => {
      const rows = [...document.querySelectorAll('.chat-forward-reactions')];
      const center = rect => rect.top + rect.height / 2;
      return rows.length > 0 && rows.every(row => {
        const bounds = row.getBoundingClientRect();
        const previous = row.previousElementSibling.getBoundingClientRect();
        return bounds.top - previous.bottom >= 7 && [...row.children].every(chip => {
          const box = chip.getBoundingClientRect();
          const emoji = chip.querySelector('img').getBoundingClientRect();
          const count = chip.querySelector('span').getBoundingClientRect();
          return box.height === (innerWidth <= 600 ? 36 : 30)
            && emoji.width === 18 && emoji.height === 18
            && Math.abs(center(emoji) - center(box)) < 0.1
            && Math.abs(center(count) - center(box)) < 0.1
            && Math.abs(count.left - emoji.right - 6) < 0.1;
        });
      });
    })()`),
    true,
    "Forwarded reactions have standard chip sizing, centered emoji/counts and a gap below content",
  );
};

try {
  assert.equal((await request("/health")).value.fixture, true);
  assert.equal((await control({ reset: true })).status, 200);
  const extraSpace = await request("/api/spaces", { name: "TEST FIXTURE Workshop" });
  assert.equal(extraSpace.status, 201);
  assert.equal((await request("/api/dms", { username: "alex" })).status, 200);
  const allDestinations = (await request("/api/chat/forward-destinations")).value.destinations;
  const extraGeneral = allDestinations.find((item) => item.spaceName === "TEST FIXTURE Workshop").id;
  const source = (await request("/api/chat/channels/chan00000003/messages")).value.messages[0];
  await control({
    incomingEdit: {
      messageId: source.id,
      text: "TEST FIXTURE — Live discussion from private planning.",
      attachments: [
        {
          id: "asset00000000001",
          kind: "file",
          contentType: "text/plain",
          name: "TEST FIXTURE notes.txt",
          size: 128,
          status: "ready",
          url: "https://example.test/fixture-notes.txt",
        },
      ],
    },
  });
  await control({ incomingReply: { rootId: source.id, text: "An existing reply before sharing." } });
  browser("open", "about:blank");
  browser("set", "viewport", "1440", "900", "2");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces`);
  // The initial voice roster grows a channel's row into a live card. Let it settle before clicking.
  wait('!!document.querySelector(".channel-select") && !!document.querySelector(".live-room")');
  browser("find", "role", "button", "click", "--name", "planning", "--exact");
  const sourceRow = `[data-message-key="${source.clientMessageId}"]`;
  wait(
    `!!document.querySelector('${sourceRow}') && !document.querySelector('.chat-initial-messages') && document.querySelector('#chat-heading')?.textContent.includes('planning')`,
  );
  browser("focus", `${sourceRow} .chat-message-actions-trigger`);
  browser("press", "Enter");
  assert.equal(
    evaluate(
      'Array.from(document.querySelectorAll(".chat-copy-actions button")).filter(button => button.textContent === "Edit message").length',
    ),
    1,
    "Source author retains the edit action",
  );
  browser("find", "role", "button", "click", "--name", "Forward message", "--exact");
  wait('document.querySelectorAll(".chat-forward-destinations input").length > 0');
  assert.equal(
    evaluate('document.querySelector(".chat-forward-original .chat-file strong")?.textContent'),
    "TEST FIXTURE notes.txt",
  );
  assert.equal(evaluate('document.querySelector(".chat-forward-send").disabled'), true);
  assert.equal(evaluate('document.querySelectorAll(".chat-forward-destinations input[type=checkbox]").length'), 5);
  assert.equal(
    evaluate(`(() => {
    const rows = [...document.querySelectorAll('.chat-forward-destinations label')];
    const first = rows[0].querySelector('strong').getBoundingClientRect().left;
    return rows.every(row => {
      const name = row.querySelector('strong').getBoundingClientRect();
      const space = row.querySelector('small').getBoundingClientRect();
      return name.left === first && space.left === first && Math.abs(space.top - name.bottom - 2) < 0.1;
    });
  })()`),
    true,
    "All names align and the space subtitle has a 2px gap, including same-space channels",
  );
  screenshot("forward-picker-default-desktop");
  const select = (id) => {
    const selector = `.chat-forward-destinations input[value="${id}"]`;
    wait(`!!document.querySelector('${selector}')`);
    browser("scrollintoview", selector);
    browser("click", selector);
  };
  select("chan00000001");
  select("chan00000002");
  // Combined space/channel tokens work in either order, with whitespace and #.
  for (const query of ["fixture studio", "DESIGN", "  #design   STUDIO ", "studio design"]) {
    browser("fill", '.chat-forward-body input[type="search"]', query);
    assert.equal(
      evaluate('document.querySelectorAll(".chat-forward-destinations input").length'),
      query === "fixture studio" ? 3 : 1,
    );
    assert.equal(evaluate('document.querySelector(".chat-forward-send").textContent'), "Forward (2)");
  }
  screenshot("forward-picker-search-desktop");
  browser("fill", '.chat-forward-body input[type="search"]', "no-such-destination");
  assert.equal(evaluate('document.querySelectorAll(".chat-forward-destinations input").length'), 0);
  assert.equal(
    evaluate('document.querySelector(".chat-forward-send").disabled'),
    false,
    "Filtering does not clear hidden selections",
  );
  screenshot("forward-picker-empty-desktop");
  // Empty fill does not dispatch React's change event in agent-browser.
  browser("focus", '.chat-forward-body input[type="search"]');
  browser("press", "Control+A");
  browser("press", "Backspace");
  select(extraGeneral);
  select("dm0000000001");
  select("dm0000000001");
  assert.equal(evaluate('document.querySelector(".chat-forward-send").textContent'), "Forward (3)");
  select("dm0000000001");
  browser("fill", ".chat-forward-body textarea", "TEST FIXTURE — Check this live conversation.");
  screenshot("forward-picker-desktop");
  browser("set", "viewport", "390", "844", "2");
  screenshot("forward-picker-mobile");
  assert.equal(evaluate("document.documentElement.scrollWidth > innerWidth"), false);
  browser("set", "viewport", "1440", "900", "2");
  // Explicitly labelled lost-response mock: the real fixture commits before the response is dropped.
  evaluate(`(() => {
    window.forwardFetch = window.fetch.bind(window); let sends = 0; window.forwardRequests = [];
    window.fetch = async (input, init) => {
      const response = await window.forwardFetch(input, init);
      if (String(input).endsWith('/forwards') && init?.method === 'POST') {
        window.forwardRequests.push({ path: String(input), ...JSON.parse(init.body) });
        if (++sends === 2) throw new TypeError('TEST FIXTURE: response lost after commit');
      }
      return response;
    };
  })()`);
  browser("click", ".chat-forward-send");
  wait('document.querySelector(".chat-forward-error")?.textContent.includes("not confirmed")');
  assert.equal(
    evaluate('document.querySelector(".chat-forward-error").textContent.includes("Forwarded to 1 destination.")'),
    true,
  );
  assert.equal(evaluate('document.querySelector(".chat-forward-send").textContent'), "Retry forwards (3)");
  assert.equal(evaluate('document.querySelector(".chat-forward-body textarea").disabled'), true);
  assert.equal(evaluate('document.querySelectorAll(".chat-forward-destinations input:checked").length'), 3);
  assert.equal(
    evaluate('document.querySelector(".chat-forward-destinations input[value=chan00000001]").checked'),
    false,
  );
  screenshot("forward-retry-desktop");
  browser("click", ".chat-forward-send");
  wait('!document.querySelector(".chat-forward-dialog")');
  const sends = evaluate("window.forwardRequests");
  assert.equal(sends.length, 5);
  assert.deepEqual(sends[1], sends[2], "Unknown outcome retries the same destination, note and UUID");
  assert.equal(new Set(sends.map((send) => send.clientMessageId)).size, 4, "One independent retry key per destination");
  for (const id of ["chan00000001", "chan00000002", extraGeneral, "dm0000000001"]) {
    const delivered = (await request(`/api/chat/channels/${id}/messages`)).value.messages.filter(
      (message) => message.forward,
    );
    assert.equal(delivered.length, 1, `Exactly one forward in ${id}, including confirmed destinations on retry`);
    assert.equal(delivered[0].content.text, "TEST FIXTURE — Check this live conversation.");
  }
  const forwards = (await request("/api/chat/channels/chan00000001/messages")).value.messages.filter(
    (message) => message.forward,
  );
  assert.equal(forwards.length, 1, "Retry does not duplicate a committed forward");
  const wrapper = forwards[0];
  assert.equal(wrapper.forward.message.id, source.id);
  assert.equal(wrapper.forward.message.thread.replyCount, 1);
  assert.equal(wrapper.forward.message.content.attachments[0].name, "TEST FIXTURE notes.txt");
  browser("focus", `${sourceRow} .chat-message-actions-trigger`);
  browser("press", "Enter");
  browser("find", "role", "button", "click", "--name", "Forward message", "--exact");
  select("chan00000001");
  select("chan00000002");
  browser("fill", ".chat-forward-body textarea", "TEST FIXTURE — Partial rejection.");
  await control({ failure: { path: "/api/chat/channels/chan00000002/forwards", method: "POST", status: 403 } });
  browser("click", ".chat-forward-send");
  wait('document.querySelector(".chat-forward-error")?.textContent.includes("Remaining forwards not sent")');
  assert.equal(evaluate('document.querySelector(".chat-forward-body textarea").disabled'), false);
  assert.equal(evaluate('document.querySelector(".chat-forward-send").textContent'), "Forward (1)");
  assert.equal(
    evaluate('document.querySelector(".chat-forward-destinations input[value=chan00000001]").checked'),
    false,
  );
  screenshot("forward-rejected-desktop");
  browser("fill", ".chat-forward-body textarea", "TEST FIXTURE — Corrected remaining note.");
  browser("click", ".chat-forward-send");
  wait('!document.querySelector(".chat-forward-dialog")');
  const afterRejection = (await request("/api/chat/channels/chan00000001/messages")).value.messages.filter(
    (item) => item.forward,
  );
  assert.equal(afterRejection.length, 2, "A definitive rejection does not reselect or resend earlier successes");
  assert.equal(afterRejection[1].content.text, "TEST FIXTURE — Partial rejection.");
  const corrected = (await request("/api/chat/channels/chan00000002/messages")).value.messages.filter(
    (item) => item.forward,
  );
  assert.equal(corrected.length, 2);
  assert.equal(corrected[1].content.text, "TEST FIXTURE — Corrected remaining note.");
  browser("find", "role", "button", "click", "--name", "general", "--exact");
  wait('!!document.querySelector(".chat-forward-card") && !document.querySelector(".chat-initial-messages")');
  const wrapperRow = `[data-message-key="${wrapper.clientMessageId}"]`;
  browser("focus", `${wrapperRow} .chat-message-actions-trigger`);
  browser("press", "Enter");
  assert.equal(
    evaluate(
      'Array.from(document.querySelectorAll(".chat-copy-actions button")).filter(button => /Edit message|Message history/.test(button.textContent)).length',
    ),
    0,
    "Even the wrapper author has no edit/history controls",
  );
  screenshot("forward-read-only-actions");
  browser("press", "Escape");
  const session = (await request("/api/chat/session", { name: "Fixture Owner" })).value;
  for (const [width, height, name] of [
    [1440, 900, "desktop"],
    [390, 844, "narrow"],
  ]) {
    browser("set", "viewport", String(width), String(height), "2");
    browser("scrollintoview", `${wrapperRow} .chat-forward-card > button`);
    browser("click", `${wrapperRow} .chat-forward-card button`);
    wait(
      `document.querySelector('#chat-heading')?.textContent.includes('planning') && !!document.querySelector('${sourceRow}.chat-message-jump-target')`,
    );
    assert.equal(
      evaluate('!!document.querySelector(".chat-forward-dialog")'),
      false,
      "Source members navigate without a modal",
    );
    assert.equal(new URL(browser("get", "url").url).searchParams.get("channel"), source.channelId);
    screenshot(`forward-source-${name}`);
    browser("open", `${web}/spaces?space=space0000001&channel=chan00000001`);
    wait(
      `!!document.querySelector('${wrapperRow} .chat-forward-card button') && !document.querySelector('.chat-initial-messages')`,
    );
  }
  browser("set", "viewport", "1440", "900", "2");
  // An outage is not a lack-of-access result and must not open the excerpt modal.
  await control({
    failure: {
      path: `/api/chat/channels/${wrapper.channelId}/forwards/${wrapper.id}/thread`,
      method: "GET",
      status: 503,
    },
  });
  browser("click", `${wrapperRow} .chat-forward-card button`);
  wait('document.querySelector(".chat-action-status")?.textContent.includes("Couldn’t open")');
  assert.equal(evaluate('!!document.querySelector(".chat-forward-dialog")'), false);
  assert.equal(evaluate('document.querySelector("#chat-heading").textContent.includes("general")'), true);
  assert.equal(
    (
      await request(
        `/api/chat/channels/${wrapper.channelId}/messages/${wrapper.id}`,
        { text: "Forbidden wrapper edit", expectedRevision: 1 },
        "owner",
        "PUT",
        session.token,
      )
    ).status,
    404,
  );
  browser("cookies", "set", "caper_fixture", "other", "--url", web, "--path", "/", "--sameSite", "Lax");
  evaluate("localStorage.clear()");
  browser("open", `${web}/spaces`);
  wait('!!document.querySelector(".chat-forward-card") && !document.querySelector(".chat-initial-messages")');
  assert.equal((await request("/api/chat/channels/chan00000003/messages", undefined, "other")).status, 404);
  assert.equal(
    (await request(`/api/chat/channels/${source.channelId}/messages/${source.id}/versions`, undefined, "other")).status,
    404,
  );
  // Let source-access resolution finish, then hold the excerpt's replies fetch.
  // The card's original stays visible while those replies fetch.
  evaluate(`(() => {
    window.forwardFetch = window.fetch.bind(window);
    let lookups = 0;
    window.fetch = (input, init) => {
      if (!String(input).includes('/forwards/') || ++lookups === 1) return window.forwardFetch(input, init);
      return new Promise(resolve => { window.releaseForward = () => resolve(window.forwardFetch(input, init)); });
    };
  })()`);
  browser("click", `${wrapperRow} .chat-forward-card button`);
  wait('!!window.releaseForward && !!document.querySelector(".chat-forward-body[aria-busy=true]")');
  assert.equal(evaluate('document.querySelectorAll(".chat-forward-dialog .chat-forward-original").length'), 1);
  assert.equal(evaluate('document.querySelector(".chat-forward-body h3, .chat-forward-status")'), null);
  assert.equal(evaluate('document.querySelector(".chat-forward-body [role=status]").className'), "sr-only");
  assert.equal(
    evaluate('document.querySelector(".chat-forward-body").textContent.includes("Heads up!")'),
    false,
    "The forwarded conversation has no update warning",
  );
  screenshot("forward-conversation-fetching-desktop");
  browser("set", "viewport", "390", "844", "2");
  screenshot("forward-conversation-fetching-mobile");
  evaluate("window.releaseForward(); window.fetch = window.forwardFetch");
  wait('document.querySelector(".chat-forward-body h3")?.textContent === "1 reply"');
  browser("press", "Escape");
  browser("set", "viewport", "1440", "900", "2");

  // A confirmed empty conversation has one empty state; unavailable originals
  // and failed requests must never resurrect the card's cached original.
  for (const mode of ["empty", "unavailable", "error"]) {
    evaluate(`(() => {
      let lookups = 0;
      window.fetch = async (input, init) => {
        if (!String(input).includes('/forwards/') || ++lookups === 1) return window.forwardFetch(input, init);
        if (${JSON.stringify(mode)} === 'error') return Response.json({ error: 'TEST FIXTURE: conversation unavailable' }, { status: 503 });
        const page = await (await window.forwardFetch(input, init)).json();
        return Response.json({ ...page, root: ${JSON.stringify(mode)} === 'unavailable' ? null : { ...page.root, thread: undefined }, messages: [], hasMore: false });
      };
    })()`);
    browser("click", `${wrapperRow} .chat-forward-card button`);
    wait('!!document.querySelector(".chat-forward-body[aria-busy=false]")');
    assert.equal(evaluate('document.querySelector(".chat-forward-body h3")'), null);
    assert.equal(
      evaluate('document.querySelectorAll(".chat-forward-dialog .chat-forward-original").length'),
      mode === "empty" ? 1 : 0,
    );
    assert.equal(
      evaluate('document.querySelector(".chat-forward-body").textContent.includes("No replies yet.")'),
      mode === "empty",
    );
    screenshot(`forward-conversation-${mode}-desktop`);
    if (mode === "empty") {
      browser("set", "viewport", "390", "844", "2");
      screenshot("forward-conversation-empty-mobile");
      browser("set", "viewport", "1440", "900", "2");
    }
    evaluate("window.fetch = window.forwardFetch");
    if (mode === "error") {
      browser("click", ".chat-forward-error button");
      wait('document.querySelector(".chat-forward-body h3")?.textContent === "1 reply"');
    }
    browser("press", "Escape");
  }
  browser("click", `${wrapperRow} .chat-forward-card button`);
  wait('document.querySelector(".chat-forward-body h3")?.textContent === "1 reply"');
  assert.equal(
    evaluate('document.querySelectorAll(".chat-forward-dialog input, .chat-forward-dialog textarea").length'),
    0,
    "Original conversation is read-only",
  );
  assert.equal(evaluate('document.querySelectorAll(".chat-forward-dialog .chat-forward-original").length'), 2);
  const saved = await request(
    `/api/chat/channels/${source.channelId}/messages/${source.id}`,
    { text: "TEST FIXTURE — The original changed after sharing.", expectedRevision: 2 },
    "owner",
    "PUT",
    session.token,
  );
  assert.equal(saved.status, 200);
  assert.equal(saved.value.revision, 3);
  for (const [emoji, userId] of [
    ["😜", "member000002"],
    ["😜", "member000001"],
    ["👀", "member000002"],
    ["👍🏽", "member000002"],
  ]) {
    await control({ incomingReaction: { channelId: source.channelId, messageId: source.id, emoji, userId } });
  }
  await control({ incomingReply: { rootId: source.id, text: "A future reply shared automatically." } });
  wait(
    'document.querySelector(".chat-forward-body h3")?.textContent === "2 replies" && document.querySelector(".chat-forward-body").textContent.includes("A future reply") && document.querySelector(".chat-forward-body").textContent.includes("The original changed") && !!document.querySelector(".chat-forward-reactions")',
  );
  assert.equal(
    evaluate('document.querySelector(".chat-forward-original .chat-file strong")?.textContent'),
    "TEST FIXTURE notes.txt",
  );
  screenshot("forward-conversation-desktop");
  checkReactionLayout();
  browser("set", "viewport", "390", "844", "2");
  screenshot("forward-conversation-mobile");
  checkReactionLayout();
  assert.equal(evaluate("document.documentElement.scrollWidth > innerWidth"), false);
  browser("press", "Escape");
  wait('!document.querySelector(".chat-forward-dialog")');
  checkReactionLayout();
  if (evaluate('!!document.querySelector(".member-list-close")')) {
    browser("click", ".member-list-close");
    // The phone member list slides out before it is removed.
    wait('!document.querySelector(".space-member-presence")');
  }
  browser("focus", `${wrapperRow} .chat-message-actions-trigger`);
  browser("press", "Enter");
  browser("find", "role", "button", "click", "--name", "Reply in thread", "--exact");
  wait('!!document.querySelector("#chat-thread-reply")');
  browser("fill", "#chat-thread-reply", "TEST FIXTURE — Replying only in the destination.");
  browser("click", '.chat-thread-send-row button[aria-label="Send reply"]');
  wait('document.querySelector(".chat-thread-summary strong")?.textContent === "1 reply"');
  const destinationThread = (
    await request(`/api/chat/channels/chan00000001/messages/${wrapper.id}/thread`, undefined, "other")
  ).value;
  assert.equal(destinationThread.messages.length, 1);
  assert.equal(destinationThread.root.forward.message.thread.replyCount, 2);
  assert.equal(
    (await request(`/api/chat/channels/chan00000003/messages/${source.id}/thread`)).value.messages.length,
    2,
  );
  screenshot("forward-destination-thread-mobile");
  // A forwarded reply uses its authorized parent, not the stripped shared snapshot.
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  const originalThread = (await request(`/api/chat/channels/${source.channelId}/messages/${source.id}/thread`)).value;
  const reply = originalThread.messages[0];
  const replyForward = (
    await request(
      `/api/chat/channels/${wrapper.channelId}/forwards`,
      {
        sourceChannelId: source.channelId,
        sourceMessageId: reply.id,
        clientMessageId: crypto.randomUUID(),
      },
      "owner",
      "POST",
      session.token,
    )
  ).value;
  browser("open", `${web}/spaces?space=space0000001&channel=chan00000001`);
  const replyForwardRow = `[data-message-key="${replyForward.clientMessageId}"]`;
  wait(
    `!!document.querySelector('${replyForwardRow} .chat-forward-card button') && !document.querySelector('.chat-initial-messages')`,
  );
  browser("click", `${replyForwardRow} .chat-forward-card button`);
  wait(
    `!!document.querySelector('[data-message-key="${reply.clientMessageId}"].chat-message-jump-target') && !!document.querySelector('#chat-thread-reply')`,
  );
  assert.equal(evaluate('!!document.querySelector(".chat-forward-dialog")'), false);
  screenshot("forward-source-reply-narrow");

  // DMs use their original channel id without a space/channel route.
  await control({ incomingMessage: { channelId: "dm0000000001", text: "Forward navigation DM source" } });
  const dmSource = (await request("/api/chat/channels/dm0000000001/messages")).value.messages.at(-1);
  const dmForward = (
    await request(
      `/api/chat/channels/${wrapper.channelId}/forwards`,
      {
        sourceChannelId: dmSource.channelId,
        sourceMessageId: dmSource.id,
        clientMessageId: crypto.randomUUID(),
      },
      "owner",
      "POST",
      session.token,
    )
  ).value;
  browser("open", `${web}/spaces?space=space0000001&channel=chan00000001`);
  const dmForwardRow = `[data-message-key="${dmForward.clientMessageId}"]`;
  wait(
    `!!document.querySelector('${dmForwardRow} .chat-forward-card button') && !document.querySelector('.chat-initial-messages')`,
  );
  browser("click", `${dmForwardRow} .chat-forward-card button`);
  wait(`!!document.querySelector('[data-message-key="${dmSource.clientMessageId}"].chat-message-jump-target')`);
  assert.equal(new URL(browser("get", "url").url).searchParams.get("dm"), dmSource.channelId);
  assert.equal(evaluate('!!document.querySelector(".chat-forward-dialog")'), false);
  screenshot("forward-source-dm-narrow");

  // A delayed lookup cannot override navigation chosen while it was pending.
  browser("set", "viewport", "1440", "900", "2");
  browser("open", `${web}/spaces?space=space0000001&channel=chan00000001`);
  wait(
    `!!document.querySelector('${wrapperRow} .chat-forward-card button') && !document.querySelector('.chat-initial-messages')`,
  );
  evaluate(`document.querySelector('${wrapperRow}').scrollIntoView({ block: 'center' })`);
  evaluate(`(() => {
    const fetch = window.fetch.bind(window);
    window.fetch = async (input, init) => {
      const response = await fetch(input, init);
      if (String(input).includes('/forwards/') && String(input).endsWith('/thread'))
        await new Promise(resolve => { window.releaseForwardLookup = resolve; });
      return response;
    };
  })()`);
  browser("click", `${wrapperRow} .chat-forward-card button`);
  wait("!!window.releaseForwardLookup");
  browser("find", "text", "design", "click", "--exact");
  evaluate("window.releaseForwardLookup()");
  wait('document.querySelector("#chat-heading")?.textContent.includes("design")');
  assert.equal(evaluate('!!document.querySelector(".chat-forward-dialog")'), false);
  console.log(
    "Forwarding browser checks passed: source navigation and focus on desktop/narrow, reply-thread and DM routes, lookup failure and stale-navigation guards; excerpt original without visible loading, single empty state, unavailable/error cache revocation and retry; aligned reaction chips and 2px subtitles, space/channel/token search, persistent multi-selection, cross-space/channel/DM delivery, partial retry deduplication, destination-only live edits/reactions/replies, read-only originals, independent destination thread.",
  );
} finally {
  browser("close");
}
