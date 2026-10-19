// Disposable loopback fixture + Chromium. Narrow/touch checks are not native-device acceptance.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const web = process.env.MESSAGE_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.MESSAGE_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const browser = (...args) => {
  const chrome = process.env.MESSAGE_TEST_CHROME ? ["--executable-path", process.env.MESSAGE_TEST_CHROME] : [];
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", "threads-check", ...chrome, ...args, "--json"], {
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
  browser("screenshot", `${artifacts}/${name}.png`);
};
const row = (id) => `[data-message-key="${id}"]`;
const channel = (id) => `.chat-panel ${row(id)}`;
const thread = (id) => `.chat-thread-panel ${row(id)}`;
const assertReplyDraft = (composer, message) => {
  const preview = evaluate(`(() => {
    const composer = document.querySelector(${JSON.stringify(composer)});
    const draft = composer.querySelector('.chat-reply-draft');
    const excerpt = draft.querySelector('.chat-reply-draft-excerpt');
    const heading = draft.querySelector('.chat-reply-draft-heading');
    const cancel = draft.querySelector('button[aria-label="Cancel reply"]').getBoundingClientRect();
    const rect = excerpt.getBoundingClientRect();
    const input = composer.querySelector('textarea').getBoundingClientRect();
    return { text: excerpt.textContent, name: heading.textContent, visible: excerpt.checkVisibility(),
      aboveInput: rect.bottom <= input.top, belowHeading: rect.top >= heading.getBoundingClientRect().bottom,
      bounded: rect.height > 0 && rect.height <= parseFloat(getComputedStyle(excerpt).lineHeight) * 2 + 1,
      fits: rect.right <= cancel.left && cancel.right <= innerWidth && document.documentElement.scrollWidth <= innerWidth,
      cancel: [cancel.width, cancel.height] };
  })()`);
  assert.equal(preview.text, message.content.text || (message.content.attachments?.length ? "Attachment" : "Message"));
  assert.equal(preview.name, `Replying to ${message.author.name}`);
  assert.equal(preview.visible, true, "The source excerpt stays visible on mobile as well as desktop");
  assert.equal(preview.aboveInput && preview.belowHeading, true, "The source sits below its author, above the input");
  assert.equal(preview.bounded && preview.fits, true, "Long quotes stay within two lines without covering Cancel");
  if (evaluate("innerWidth <= 760 || matchMedia('(pointer: coarse)').matches"))
    assert.ok(
      preview.cancel.every((size) => size >= 44),
      "Cancel reply retains its touch target",
    );
};
const alignedHeaders = (width) => {
  evaluate("document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))");
  const geometry = evaluate(`(() => {
    const main = document.querySelector('.chat-heading').getBoundingClientRect();
    const heading = document.querySelector('.chat-thread-heading').getBoundingClientRect();
    const panel = document.querySelector('.chat-thread-panel').getBoundingClientRect();
    const name = document.querySelector('.chat-thread-heading span').getBoundingClientRect();
    const control = document.querySelector(innerWidth > 760 ? '.chat-thread-close' : '.chat-thread-back').getBoundingClientRect();
    return { main: [main.top, main.bottom, main.height], thread: [heading.top, heading.bottom, heading.height],
      contentFits: (innerWidth > 760 ? name.right <= control.left : control.right <= name.left) && name.bottom <= heading.bottom && control.right <= heading.right && name.right <= heading.right,
      panelFits: heading.left >= panel.left && heading.right <= panel.right && panel.right <= innerWidth,
      control: [control.width, control.height] };
  })()`);
  assert.equal(geometry.thread[2], 54, `Thread header stays 54px at ${width}px`);
  assert.equal(geometry.main[2], 54, `Channel header stays 54px at ${width}px`);
  if (width > 760) assert.deepEqual(geometry.thread, geometry.main, `Header dividers align at ${width}px`);
  assert.equal(geometry.contentFits, true, `Channel name and close/back control fit at ${width}px`);
  assert.equal(geometry.panelFits, true, `Header stays inside the visible panel at ${width}px`);
  if (width <= 760) assert.deepEqual(geometry.control, [44, 44], "Mobile Back keeps its touch target");
};
const assertReplyAction = (expected) => {
  wait('!!document.querySelector(".chat-message-actions")');
  const actions = evaluate(
    '[...document.querySelectorAll(".chat-message-actions button")].map(button => button.textContent.trim())',
  );
  assert.equal(actions.includes("Reply in thread"), expected);
  for (const action of ["Pin message", "Copy text", "Copy message ID"]) assert.ok(actions.includes(action));
  assert.equal(evaluate('!!document.querySelector(".chat-quick-reactions")'), true);
};
const actions = (selector, expected) => {
  browser("scrollintoview", selector);
  browser("click", `${selector} .chat-message-actions-trigger`);
  assertReplyAction(expected);
};
const dismissActions = () => {
  browser("find", "role", "button", "click", "--name", "Close message actions", "--exact");
  wait('!document.querySelector(".chat-message-actions")');
};
const headers = { authorization: "Bearer fixture-owner-token", connection: "close" };
const history = async () => (await fetch(`${api}/api/chat/channels/chan00000001/messages`, { headers })).json();
const close = () => {
  browser("press", "Escape");
  wait('!document.querySelector(".chat-thread-panel") && !history.state?.caperThread');
};
const open = (id) => {
  const trigger = evaluate(
    `${JSON.stringify([`${channel(id)} .chat-reply-thread`, `${channel(id)} .chat-thread-summary`])}.find(selector => document.querySelector(selector)?.checkVisibility({ visibilityProperty: true }))`,
  );
  if (trigger) {
    browser("focus", trigger);
    browser("click", trigger);
  } else {
    actions(channel(id), true);
    browser("find", "role", "button", "click", "--name", "Reply in thread", "--exact");
  }
  wait(
    '!!document.querySelector(".chat-thread-panel") && !document.querySelector(".chat-thread-messages[aria-busy=true]")',
  );
  evaluate(
    "Promise.all(document.querySelector('.chat-thread-panel').getAnimations().map(animation => animation.finished))",
  );
  assert.deepEqual(
    evaluate(`(() => {
      const replies = document.querySelector('.chat-thread-messages');
      return [getComputedStyle(replies).scrollbarWidth, getComputedStyle(replies, '::-webkit-scrollbar').display];
    })()`),
    ["none", "none"],
    "Thread replies keep scrolling without revealing a scrollbar",
  );
};
const reply = (text) => {
  browser("fill", "#chat-thread-reply", text);
  browser("click", '.chat-thread-send-row button[aria-label="Send reply"]');
  wait(
    '!document.querySelector(".chat-thread-panel .chat-message-pending") && document.querySelector("#chat-thread-reply").value === ""',
  );
};

async function checkReactionFollow(root, layout) {
  const control = async (body) => {
    const response = await fetch(`${api}/__fixture/control`, {
      method: "POST",
      headers: { "content-type": "application/json", connection: "close" },
      body: JSON.stringify(body),
    });
    assert.equal(response.status, 200);
  };
  for (let index = 0; index < 16; index++)
    await control({ incomingReply: { rootId: root.id, text: `Reaction scroll fixture ${index}.\nSecond line.` } });
  const page = await (
    await fetch(`${api}/api/chat/channels/chan00000001/messages/${root.id}/thread`, { headers })
  ).json();
  const last = page.messages.at(-1);
  wait(`!!document.querySelector('${thread(last.clientMessageId)}')`);
  evaluate("new Promise(r => setTimeout(r, 300))");
  const visible = `(() => {
    const bounds = document.querySelector('.chat-thread-messages').getBoundingClientRect();
    const chips = [...document.querySelectorAll('${thread(last.clientMessageId)} .chat-reaction')];
    return chips.length > 0 && chips.every(chip => {
      const rect = chip.getBoundingClientRect();
      return rect.top >= bounds.top && rect.bottom <= bounds.bottom;
    });
  })()`;
  for (const [index, emoji] of [
    "👍",
    "🎉",
    "🚀",
    "❤️",
    "😂",
    "🔥",
    "👏",
    "👀",
    "✅",
    "💯",
    "🙌",
    "🤔",
    "⭐",
  ].entries()) {
    await control({ incomingReaction: { channelId: "chan00000001", messageId: last.id, emoji } });
    wait(`document.querySelectorAll('${thread(last.clientMessageId)} .chat-reaction').length === ${index + 1}`);
    wait(visible);
  }
  screenshot(`thread-reaction-${layout}-latest`);
  const before = evaluate(`(() => {
    const scroller = document.querySelector('.chat-thread-messages');
    scroller.scrollTop -= 400;
    scroller.dispatchEvent(new Event('scroll', { bubbles: true }));
    return scroller.scrollTop;
  })()`);
  await control({ incomingReaction: { channelId: "chan00000001", messageId: last.id, emoji: "🦀" } });
  wait(`document.querySelectorAll('${thread(last.clientMessageId)} .chat-reaction').length === 14`);
  evaluate("new Promise(r => setTimeout(r, 300))");
  assert.equal(evaluate("document.querySelector('.chat-thread-messages').scrollTop"), before);
  console.log(`PASS: ${layout}: first/wrapped thread reactions visible and history reader retained.`);
}

const checkThreadResize = (rootID) => {
  // This scenario needs room for a 600px thread plus the channel. Don't depend
  // on the default/remembered Members visibility to provide that space.
  if (evaluate('!!document.querySelector("#space-member-list")')) {
    browser("find", "role", "button", "click", "--name", "Hide member list", "--exact");
    wait('!document.querySelector("#space-member-list")');
  }
  const handle = ".chat-thread-resize";
  const width = () => evaluate('document.querySelector(".chat-thread-panel").getBoundingClientRect().width');
  const drag = (delta) => {
    const point = evaluate(
      `(() => { const r = document.querySelector('${handle}').getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + 120 }; })()`,
    );
    browser("mouse", "move", String(point.x), String(point.y));
    browser("mouse", "down", "left");
    browser("mouse", "move", String(point.x + delta), String(point.y));
    browser("mouse", "up", "left");
  };
  browser("dblclick", handle);
  assert.equal(width(), 340);
  browser("fill", "#chat-thread-reply", "TEST FIXTURE — resizing keeps the draft");
  drag(-83);
  assert.equal(width(), 423, "Dragging the left edge left widens the actual thread");
  assert.equal(evaluate(`document.querySelector('${handle}').getAttribute('aria-valuenow')`), "423");
  assert.equal(
    evaluate('document.querySelector("#chat-thread-reply").value'),
    "TEST FIXTURE — resizing keeps the draft",
  );
  browser("mouse", "move", "1400", "30");
  screenshot("threads-desktop-resized");
  drag(600);
  assert.equal(width(), 300, "Pointer capture works outside the moving handle and clamps at minimum");
  browser("focus", handle);
  browser("press", "ArrowLeft");
  assert.equal(width(), 310);
  browser("press", "ArrowRight");
  assert.equal(width(), 300);
  browser("press", "ArrowLeft");
  browser("press", "Home");
  assert.equal(width(), 300, "Home selects the minimum");
  browser("press", "End");
  assert.equal(width(), 600);
  browser("press", "ArrowLeft");
  assert.equal(width(), 600, "Keyboard cannot exceed the maximum");
  browser("set", "viewport", "1024", "900", "2");
  wait('Number(document.querySelector(".chat-thread-resize").getAttribute("aria-valuemax")) < 600');
  const room = evaluate('document.querySelector(".chat-layout").clientWidth');
  assert.equal(width(), Math.floor(room - 300), "A smaller window reserves 300px for the channel");
  assert.ok(evaluate('document.querySelector(".chat-panel").getBoundingClientRect().width') >= 300);
  screenshot("threads-desktop-constrained");
  browser("set", "viewport", "1440", "900", "2");
  wait('document.querySelector(".chat-thread-panel").getBoundingClientRect().width === 600');
  close();
  open(rootID);
  assert.equal(width(), 600, "Reopening and temporary window constraints preserve the preferred width");
  browser("dblclick", handle);
  assert.equal(width(), 340);
  drag(-83);
  assert.equal(width(), 423);
  browser("fill", "#chat-thread-reply", "");
};

const checkReplyDelivery = async (root, other, layout) => {
  // Hold before fixture delivery so neither HTTP nor gateway can confirm the reply.
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.replyRequests = [];
    window.restoreReplyFetch = () => { window.fetch = original; };
    window.fetch = async (input, init) => {
      if (init?.method !== 'POST' || !String(input).endsWith('/messages')) return original(input, init);
      window.replyRequests.push(JSON.parse(init.body));
      const status = await new Promise(resolve => { window.releaseReply = resolve; });
      return status === 200 ? original(input, init) : Response.json({ error: 'TEST FIXTURE: reply delivery problem' }, { status });
    };
  })()`);
  for (const status of [503, 422]) {
    open(root.clientMessageId);
    browser("fill", "#chat-thread-reply", `TEST FIXTURE — ${layout} reply ${status}`);
    browser(layout === "desktop" ? "check" : "uncheck", ".chat-thread-send-row input");
    browser("click", '.chat-thread-send-row button[aria-label="Send reply"]');
    wait('!!document.querySelector(".chat-thread-panel .chat-message-pending")');
    assert.equal(evaluate('document.querySelector(".chat-panel .chat-inline-error")'), null, "Normal sends are quiet");
    if (status === 503) screenshot(`threads-${layout}-sending`);
    close();
    assert.equal(
      evaluate('document.querySelector(".chat-panel .chat-inline-error")'),
      null,
      "Closing while sending is quiet",
    );
    evaluate(`window.releaseReply(${status})`);
    const notice = status === 422 ? "A thread reply wasn’t sent." : "A thread reply couldn’t be confirmed.";
    wait(`document.querySelector('.chat-panel .chat-inline-error')?.textContent.includes(${JSON.stringify(notice)})`);
    if (status === 503) screenshot(`threads-${layout}-reply-problem`);
    open(other.clientMessageId);
    assert.ok(
      evaluate('document.querySelector(".chat-panel .chat-inline-error")?.textContent.includes("Review reply")'),
      "Another thread must not hide recovery",
    );
    close();
    browser("find", "role", "button", "click", "--name", "Review reply", "--exact");
    wait(
      `!!document.querySelector('${thread(root.clientMessageId)}') && !document.querySelector('.chat-thread-messages[aria-busy=true]')`,
    );
    assert.equal(
      evaluate('document.querySelector(".chat-panel .chat-inline-error")'),
      null,
      "An open reply owns its error",
    );
    assert.ok(
      evaluate(
        'document.querySelector(".chat-thread-panel [role=alert]")?.textContent.includes("TEST FIXTURE: reply delivery problem")',
      ),
    );
    if (status === 422) {
      browser("find", "role", "button", "click", "--name", "Edit", "--exact");
      wait('!document.querySelector(".chat-thread-panel .chat-message-pending")');
      assert.equal(
        evaluate('document.querySelector("#chat-thread-reply").value'),
        `TEST FIXTURE — ${layout} reply ${status}`,
      );
      browser("click", '.chat-thread-send-row button[aria-label="Send reply"]');
      wait('!!document.querySelector(".chat-thread-panel .chat-message-pending")');
      evaluate("window.releaseReply(422)");
      wait('!!document.querySelector(".chat-thread-panel [role=alert]")');
      browser("find", "role", "button", "click", "--name", "Dismiss", "--exact");
      wait('!document.querySelector(".chat-thread-panel .chat-message-pending")');
      close();
    } else {
      const command = evaluate("window.replyRequests.at(-1)");
      assert.equal(command.threadRootId, root.id);
      assert.equal(command.broadcast, layout === "desktop");
      assert.equal(command.text, `TEST FIXTURE — ${layout} reply ${status}`);
      browser("find", "role", "button", "click", "--name", "Retry send", "--exact");
      wait('!document.querySelector(".chat-thread-panel [role=alert]")');
      assert.deepEqual(evaluate("window.replyRequests.at(-1)"), command, "Retry freezes ID, text, root and broadcast");
      close();
      open(other.clientMessageId);
      assert.equal(evaluate('document.querySelector(".chat-panel .chat-inline-error")'), null);
      assert.equal(
        evaluate('document.querySelector(".chat-thread-panel .chat-inline-error")'),
        null,
        "A send in another thread is not an error",
      );
      close();
      evaluate("window.releaseReply(200)");
      open(root.clientMessageId);
      wait(
        `!document.querySelector('.chat-thread-panel .chat-message-pending') && !!document.querySelector('${thread(command.clientMessageId)}')`,
      );
      const page = await (
        await fetch(`${api}/api/chat/channels/chan00000001/messages/${root.id}/thread`, { headers })
      ).json();
      assert.equal(page.messages.filter((message) => message.clientMessageId === command.clientMessageId).length, 1);
      assert.equal(
        (await history()).messages.filter((message) => message.clientMessageId === command.clientMessageId).length,
        layout === "desktop" ? 1 : 0,
      );
      close();
    }
    assert.equal(
      evaluate('document.querySelector(".chat-panel .chat-inline-error")'),
      null,
      "Recovery clears the notice",
    );
  }
  evaluate("window.restoreReplyFetch()");
};

const checkLongQuotePreview = async (layout) => {
  // Reloads restore Members; dismiss its mobile overlay before testing quotes.
  if (evaluate('!!document.querySelector("#space-member-list")')) {
    if (layout === "mobile") browser("click", "#space-member-list .member-list-close");
    else browser("find", "role", "button", "click", "--name", "Hide member list", "--exact");
    wait('!document.querySelector("#space-member-list")');
  }
  const session = async (token) => {
    const auth = { authorization: `Bearer ${token}`, "content-type": "application/json", connection: "close" };
    const response = await fetch(`${api}/api/chat/session`, { method: "POST", headers: auth, body: "{}" });
    assert.equal(response.status, 200);
    return { ...auth, "x-caper-chat-token": (await response.json()).token };
  };
  const peer = await session("fixture-member-token");
  const own = await session("fixture-owner-token");
  const fullText =
    "TEST FIXTURE — 🙂🚀 Replies bring an earlier message back into the conversation without moving the original.\nThe quote preserves the original wording and line breaks, even if the source is edited later. This longer paragraph makes the three-line preview wrap on both wide and narrow screens.\nShow more reveals the complete saved quote; View original jumps to the live message.\nhttps://example.test/" +
    "a-long-unbroken-path-".repeat(9) +
    "END-OF-QUOTE";
  const preview =
    "TEST FIXTURE — 🙂🚀 Replies bring an earlier message back into the conversation without moving the ori…";
  const send = async (auth, text, replyToId, clientMessageId = randomUUID()) => {
    const response = await fetch(`${api}/api/chat/channels/chan00000001/messages`, {
      method: "POST",
      headers: auth,
      body: JSON.stringify({ text, clientMessageId, ...(replyToId ? { replyToId } : {}) }),
    });
    assert.equal(response.status, 200);
    return response.json();
  };
  const source = await send(peer, fullText);
  wait(
    '!document.querySelector(".chat-initial-messages") && !!document.querySelector(".chat-panel .chat-scroller[aria-hidden=false]")',
  );
  browser("focus", ".chat-panel .chat-scroller[aria-hidden=false]");
  browser("press", "End");
  wait(`!!document.querySelector('${channel(source.clientMessageId)}')`);
  actions(channel(source.clientMessageId), true);
  browser("find", "role", "button", "click", "--name", "Reply", "--exact");
  wait('!!document.querySelector(".chat-composer .chat-reply-draft")');
  assertReplyDraft(".chat-composer", source);
  browser("fill", ".chat-composer textarea", "TEST FIXTURE — Keep this unsent text.");
  screenshot(`reply-draft-${layout}`);
  browser("find", "role", "button", "click", "--name", "Cancel reply", "--exact");
  assert.equal(
    evaluate('document.querySelector(".chat-composer textarea").value'),
    "TEST FIXTURE — Keep this unsent text.",
  );
  assert.equal(evaluate('!!document.querySelector(".chat-composer .chat-reply-draft")'), false);
  browser("fill", ".chat-composer textarea", "");
  const sibling = await send(
    own,
    "TEST FIXTURE — Following up on this message, with the full original available in the quote.",
    source.id,
    `00000000-0000-4000-8000-0000000000${layout === "desktop" ? "11" : "22"}`,
  );
  const quoted = await send(
    own,
    "TEST FIXTURE — Following up on this message, with the full original available in the quote.",
    source.id,
    `00000000-0000-4000-8000-0000000000${layout === "desktop" ? "a7" : "e9"}`,
  );
  assert.equal(quoted.replyTo.fullText, fullText);
  assert.equal([...quoted.replyTo.text].length, 200);
  const selector = `.chat-panel .chat-scroller ${row(quoted.clientMessageId)}`;
  // Source navigation and a viewport change can leave the virtualized list at
  // a history anchor. Use the normal latest-message shortcut, not its temporary
  // hydration preview, before interacting with the newly delivered quote.
  wait(
    '!document.querySelector(".chat-initial-messages") && !!document.querySelector(".chat-panel .chat-scroller[aria-hidden=false]")',
  );
  browser("focus", ".chat-panel .chat-scroller[aria-hidden=false]");
  browser("press", "End");
  wait(`!!document.querySelector('${selector} .chat-reply-expand')`);
  browser("scrollintoview", selector);
  const geometry = () =>
    evaluate(`(() => {
    const row = document.querySelector('${selector}');
    const quote = row.querySelector('.chat-reply-context');
    const excerpt = quote.querySelector('.chat-reply-excerpt');
    const avatar = row.querySelector(':scope > .chat-avatar').getBoundingClientRect();
    const line = quote.querySelector('svg').getBoundingClientRect();
    const source = quote.querySelector('.chat-reply-avatar').getBoundingClientRect();
    const beam = quote.querySelector('.chat-reply-route');
    const end = beam.getPointAtLength(beam.getTotalLength()).matrixTransform(beam.getScreenCTM());
    const start = beam.getPointAtLength(0).matrixTransform(beam.getScreenCTM());
    const sourceSocket = quote.querySelector('.chat-reply-socket-source').getScreenCTM();
    const targetSocket = quote.querySelector('.chat-reply-socket-target').getScreenCTM();
    const sourceMouth = new DOMPoint(0, 0).matrixTransform(sourceSocket);
    const targetMouth = new DOMPoint(0, 0).matrixTransform(targetSocket);
    const sourceBack = new DOMPoint(-12, 0).matrixTransform(sourceSocket);
    const targetBack = new DOMPoint(-12, 0).matrixTransform(targetSocket);
    const card = quote.querySelector('.chat-reply-body').getBoundingClientRect();
    const expand = quote.querySelector('.chat-reply-expand').getBoundingClientRect();
    const excerptBounds = excerpt.getBoundingClientRect();
    const heading = quote.querySelector('.chat-reply-heading').getBoundingClientRect();
    return { clamp: getComputedStyle(excerpt).webkitLineClamp, mask: getComputedStyle(excerpt).maskImage, height: excerpt.clientHeight, natural: excerpt.scrollHeight,
      expandCenter: Math.abs(expand.left + expand.width / 2 - card.left - card.width / 2), expandHeight: expand.height,
      fadeCenter: Math.abs(expand.top + expand.height / 2 - excerptBounds.top - excerptBounds.height * 0.7),
      footerHeight: card.bottom - excerptBounds.bottom, headingGap: expand.top - heading.bottom,
      connection: Math.abs(line.bottom - avatar.top - avatar.height / 2), overflow: document.documentElement.scrollWidth > innerWidth,
      sourceConnection: Math.hypot(sourceMouth.x - source.left, sourceMouth.y - source.top - source.height / 2),
      replyConnection: Math.hypot(targetMouth.x - avatar.left, targetMouth.y - avatar.top - avatar.height / 2),
      wireConnection: Math.max(Math.hypot(end.x - sourceBack.x, end.y - sourceBack.y), Math.hypot(start.x - targetBack.x, start.y - targetBack.y)),
      separation: avatar.top - quote.querySelector('.chat-reply-body').getBoundingClientRect().bottom,
      quoteHeight: quote.getBoundingClientRect().height };
  })()`);
  assert.equal(evaluate(`document.querySelector('${selector} .chat-reply-excerpt').textContent`), preview);
  const collapsed = geometry();
  assert.equal(collapsed.clamp, "3");
  assert.ok(collapsed.height <= 54);
  assert.notEqual(collapsed.mask, "none", "The collapsed quote fades into its dark card");
  assert.ok(collapsed.expandCenter < 1, "Show more is centered in the quote card");
  assert.ok(collapsed.fadeCenter < 1, "Show more floats at the midpoint of the lower-60% fade");
  assert.ok(collapsed.footerHeight <= 12, "Show more does not allocate a separate footer row");
  assert.ok(collapsed.headingGap >= 1, "The floating pill stays below the original author heading");
  assert.ok(
    collapsed.expandHeight >= (layout === "desktop" ? 32 : 44),
    "The expansion button keeps its pointer/touch target",
  );
  assert.ok(collapsed.connection < 1, "The connector meets the actual reply avatar");
  assert.ok(
    collapsed.sourceConnection < 1 && collapsed.replyConnection < 1,
    "Both avatar attachments match their rendered boundaries",
  );
  assert.ok(collapsed.wireConnection < 0.1, "The string stays attached to both cup backs");
  assert.ok(collapsed.separation >= 8, "The quote card stays separate from the replying author");
  assert.equal(collapsed.overflow, false);
  screenshot(`reply-preview-${layout}-collapsed`);
  browser("click", `${selector} .chat-reply-expand`);
  wait(`document.querySelector('${selector} .chat-reply-expand')?.getAttribute('aria-expanded') === 'true'`);
  const expanded = geometry();
  assert.equal(evaluate(`document.querySelector('${selector} .chat-reply-excerpt').textContent`), fullText);
  assert.equal(expanded.mask, "none", "The expanded original has no fade");
  assert.equal(expanded.height, expanded.natural, "Expansion reveals the full saved text");
  assert.ok(expanded.quoteHeight > collapsed.quoteHeight);
  assert.ok(expanded.expandCenter < 1, "Show less stays centered after expansion");
  assert.ok(expanded.connection < 1, "Expansion does not detach the connector");
  assert.ok(expanded.sourceConnection < 1 && expanded.replyConnection < 1);
  assert.ok(expanded.wireConnection < 0.1);
  assert.ok(expanded.separation >= 8);
  assert.equal(expanded.overflow, false, "Long URLs wrap instead of overflowing");
  browser("scrollintoview", selector);
  screenshot(`reply-preview-${layout}-expanded`);
  browser("click", `${selector} .chat-reply-expand`);
  wait(`document.querySelector('${selector} .chat-reply-expand')?.getAttribute('aria-expanded') === 'false'`);
  browser("mouse", "move", "10", "10");
  evaluate("document.activeElement.blur()");
  browser("wait", "2000");
  const sampleScroll = () =>
    evaluate(`new Promise(resolve => {
      const quote = document.querySelector('${selector} .chat-reply-context');
      const scroller = quote.closest('.chat-scroller');
      const cup = quote.querySelector('.chat-reply-socket-target');
      const sibling = scroller.querySelector('${row(sibling.clientMessageId)} .chat-reply-socket-target');
      const wire = quote.querySelector('.chat-reply-route');
      const initial = wire.getAttribute('d');
      const samples = []; const start = performance.now();
      const top = scroller.scrollTop;
      scroller.scrollTop += top > 80 ? -64 : 64;
      const sample = () => {
        const rotation = Math.atan2(cup.getCTM().b, cup.getCTM().a);
        const siblingRotation = Math.atan2(sibling.getCTM().b, sibling.getCTM().a);
        const back = new DOMPoint(-12, 0).matrixTransform(cup.getScreenCTM());
        const end = wire.getPointAtLength(0).matrixTransform(wire.getScreenCTM());
        samples.push({ rotation, siblingRotation, attached: Math.hypot(back.x - end.x, back.y - end.y), changed: wire.getAttribute('d') !== initial });
        if (performance.now() - start < 2000) requestAnimationFrame(sample);
        else resolve({ samples, initial, final: wire.getAttribute('d'), moved: Math.abs(scroller.scrollTop - top) });
      }; requestAnimationFrame(sample);
    })`);
  const motion = sampleScroll();
  assert.ok(motion.moved >= 60, "The real timeline scrolled");
  assert.ok(
    motion.samples.some((sample) => Math.abs(sample.rotation) > 0.008 && sample.changed),
    "Scrolling flexes the string and tilts the cups",
  );
  assert.ok(
    motion.samples.some((sample) => Math.abs(sample.siblingRotation) > 0.008),
    "Both replies move",
  );
  assert.ok(
    motion.samples.some((sample) => Math.abs(sample.rotation - sample.siblingRotation) > 0.005),
    "Replies to the same original respond independently to the same scroll",
  );
  assert.ok(
    motion.samples.every((sample) => sample.attached < 0.1),
    "The string remains attached throughout the spring",
  );
  assert.equal(motion.final, motion.initial, "The connection settles exactly back to rest");
  assert.ok(
    motion.samples
      .slice(-10)
      .every((sample) => sample.rotation === 0 && sample.siblingRotation === 0 && !sample.changed),
    "No idle jiggling",
  );
  browser("set", "media", "dark", "reduced-motion");
  const reduced = sampleScroll();
  assert.ok(reduced.moved >= 60);
  assert.ok(
    reduced.samples.every((sample) => sample.rotation === 0 && sample.siblingRotation === 0 && !sample.changed),
    "Reduced-motion connections stay still while scrolling",
  );
  browser("set", "media", "dark");
  assert.equal(evaluate("matchMedia('(prefers-reduced-motion: reduce)').matches"), false);
  const edited = "TEST FIXTURE — The original was edited after this reply was sent.";
  const edit = await fetch(`${api}/api/chat/channels/chan00000001/messages/${source.id}`, {
    method: "PUT",
    headers: peer,
    body: JSON.stringify({ text: edited, expectedRevision: 1 }),
  });
  assert.equal(edit.status, 200);
  wait(`document.querySelector('${channel(source.clientMessageId)} p')?.textContent === ${JSON.stringify(edited)}`);
  assert.equal(
    evaluate(`document.querySelector('${selector} .chat-reply-excerpt').textContent`),
    preview,
    "Source edits do not rewrite a quote",
  );
  browser("click", `${selector} .chat-reply-view`);
  wait(`document.querySelector('${channel(source.clientMessageId)}')?.classList.contains('chat-message-jump-target')`);
  const block = await fetch(`${api}/api/blocks/member000001`, { method: "PUT", headers: own, body: "{}" });
  assert.equal(block.status, 204);
  browser("open", `${web}/spaces`);
  wait(`document.querySelector('${selector} .chat-reply-context')?.textContent.includes('Blocked message')`);
  assert.equal(
    evaluate(
      `!!document.querySelector('${selector} .chat-reply-excerpt, ${selector} .chat-reply-expand, ${selector} .chat-reply-avatar, ${selector} .chat-reply-connector')`,
    ),
    false,
    "Blocked quote text, expansion and identity stay hidden",
  );
  const unblock = await fetch(`${api}/api/blocks/member000001`, { method: "DELETE", headers: own });
  assert.equal(unblock.status, 204);
  browser("open", `${web}/spaces`);
  wait(`!!document.querySelector('${selector} .chat-reply-expand')`);
  console.log(
    `PASS: ${layout}: 100-scalar faded preview, full immutable expansion, both avatar sockets and string attachments, separate quote card, URL containment, source jump and blocked-source privacy; independent scroll-driven springs for replies to the same source, attached endpoints during motion, exact settling, no idle jiggling and static reduced motion.`,
  );
};

let socket;
try {
  assert.equal((await (await fetch(`${api}/health`, { headers: { connection: "close" } })).json()).fixture, true);
  const reset = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify({ reset: true }),
  });
  assert.equal(reset.status, 200);
  const initial = await history();
  const root = initial.messages.at(-1);
  const other = initial.messages[1];
  browser("open", "about:blank");
  browser("set", "viewport", "1440", "900", "2");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces`);
  evaluate('localStorage.removeItem("caper:thread-panel-width")');
  browser("open", `${web}/spaces`);
  wait(
    `!!document.querySelector('${channel(root.clientMessageId)}') && !document.querySelector('.chat-initial-messages')`,
  );
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.threadRequests = [];
    window.fetch = (input, init) => {
      if (String(input).includes('/thread')) window.threadRequests.push(String(input));
      return original(input, init);
    };
  })()`);
  if (process.env.MESSAGE_TEST_CHROME) {
    assert.equal(evaluate('matchMedia("(hover: hover) and (pointer: fine)").matches'), true);
    browser("hover", "#chat-heading");
    assert.equal(
      evaluate(
        `document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread').checkVisibility({ opacityProperty: true })`,
      ),
      false,
    );
    browser("focus", `${channel(root.clientMessageId)} .chat-reply-thread`);
    wait(
      `document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread').checkVisibility({ opacityProperty: true })`,
    );
    assert.equal(
      evaluate(
        `document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread').checkVisibility({ opacityProperty: true })`,
      ),
      true,
    );
    assert.deepEqual(
      evaluate(
        `['.chat-reply-thread', '.chat-reply-message', '.chat-add-reaction', '.chat-message-actions-trigger'].map(selector => { const rect = document.querySelector('${channel(root.clientMessageId)} ' + selector).getBoundingClientRect(); return [rect.width, rect.height]; })`,
      ),
      [
        [24, 24],
        [24, 24],
        [24, 24],
        [24, 24],
      ],
    );
  }
  browser("click", `${channel(root.clientMessageId)} .chat-reply-message`);
  wait('!!document.querySelector(".chat-composer .chat-reply-draft")');
  assertReplyDraft(".chat-composer", root);
  assert.equal(evaluate('document.activeElement === document.querySelector(".chat-composer textarea")'), true);
  assert.equal(evaluate('!!document.querySelector(".chat-thread-panel")'), false, "Reply does not open a thread");
  browser("find", "role", "button", "click", "--name", "Cancel reply", "--exact");
  actions(channel(root.clientMessageId), true);
  dismissActions();
  open(root.clientMessageId);
  assert.equal(evaluate("document.activeElement.id"), "chat-thread-reply");
  assert.equal(evaluate('document.querySelector(".chat-thread-send-row input").checked'), false);
  assert.equal(
    evaluate(
      `document.querySelector('${channel(root.clientMessageId)}').classList.contains('chat-message-thread-active')`,
    ),
    true,
  );
  assert.equal(evaluate('document.querySelector(".chat-thread-panel").hasAttribute("aria-modal")'), false);
  assert.equal(
    evaluate('document.querySelector(".chat-thread-status").textContent'),
    "No replies yet. Start the thread.",
  );
  alignedHeaders(1440);
  screenshot("threads-desktop-empty");
  // Explicit layout-only mock: the longest channel name must not grow the header.
  const originalName = evaluate('document.querySelector(".chat-thread-heading span").textContent');
  evaluate('document.querySelector(".chat-thread-heading span").textContent = "in #" + "long-channel-name-".repeat(3)');
  for (const width of [1024, 761]) {
    browser("set", "viewport", String(width), "900", "2");
    alignedHeaders(width);
    if (width === 1024) screenshot("threads-desktop-long-name");
  }
  browser("set", "viewport", "1440", "900", "2");
  evaluate(`document.querySelector(".chat-thread-heading span").textContent = ${JSON.stringify(originalName)}`);
  actions(thread(root.clientMessageId), false);
  dismissActions();
  browser("click", `${thread(root.clientMessageId)} .chat-reply-message`);
  wait('!!document.querySelector(".chat-thread-panel .chat-reply-draft")');
  assertReplyDraft(".chat-thread-panel", root);
  screenshot("reply-draft-thread-desktop");
  assert.equal(evaluate("document.activeElement.id"), "chat-thread-reply");
  reply("TEST FIXTURE — Keep the layout discussion here. 🙂");
  const quoted = (
    await (await fetch(`${api}/api/chat/channels/chan00000001/messages/${root.id}/thread`, { headers })).json()
  ).messages.at(-1);
  assert.equal(quoted.replyTo.id, root.id, "The quick reply action quotes the selected thread message");
  browser("click", `${thread(quoted.clientMessageId)} .chat-reply-context`);
  wait(
    `!document.querySelector('.chat-thread-panel, .chat-initial-messages') && document.querySelector('${channel(root.clientMessageId)}')?.classList.contains('chat-message-jump-target') && document.querySelector('${channel(root.clientMessageId)}')?.checkVisibility({ visibilityProperty: true })`,
  );
  open(root.clientMessageId);
  wait(
    `document.querySelector('${channel(root.clientMessageId)} .chat-thread-summary strong')?.textContent === '1 reply'`,
  );
  assert.ok(
    evaluate(`(() => {
      const summary = document.querySelector('${channel(root.clientMessageId)} .chat-thread-summary');
      const text = summary.parentElement.querySelector('p');
      return summary.getBoundingClientRect().top - text.getBoundingClientRect().bottom <= 4;
    })()`),
    "Thread summary stays close to the message instead of reserving a blank line",
  );
  if (process.env.MESSAGE_TEST_CHROME) {
    browser("hover", `${channel(root.clientMessageId)} .chat-thread-summary`);
    evaluate(
      `Promise.all(document.querySelector('${channel(root.clientMessageId)} .chat-thread-summary').getAnimations().map(animation => animation.finished))`,
    );
    assert.notEqual(
      evaluate(
        `getComputedStyle(document.querySelector('${channel(root.clientMessageId)} .chat-thread-summary')).backgroundColor`,
      ),
      "rgba(0, 0, 0, 0)",
      "The whole thread reply control highlights on hover, not just its count",
    );
  }
  assert.equal(
    evaluate('document.querySelectorAll(".chat-panel .chat-message").length'),
    initial.messages.length,
    "Thread-only replies stay out of the channel",
  );
  assert.equal(
    evaluate(`document.querySelectorAll('${channel(root.clientMessageId)} .chat-thread-avatars > span').length`),
    1,
  );
  browser("click", ".chat-thread-send-row input");
  reply("TEST FIXTURE — This update is also visible in the channel.");
  const page = await (
    await fetch(`${api}/api/chat/channels/chan00000001/messages/${root.id}/thread`, { headers })
  ).json();
  assert.equal(page.messages.length, 2);
  const broadcast = page.messages.at(-1);
  assert.equal(broadcast.broadcast, true);
  wait(
    `!!document.querySelector('${channel(broadcast.clientMessageId)}') && !!document.querySelector('${thread(broadcast.clientMessageId)}')`,
  );
  assert.equal((await history()).messages.filter((message) => message.id === broadcast.id).length, 1);
  // The same reply keeps its action in the channel, but loses it inside the thread.
  actions(channel(broadcast.clientMessageId), true);
  screenshot("thread-actions-desktop-channel");
  dismissActions();
  actions(thread(broadcast.clientMessageId), false);
  screenshot("thread-actions-desktop-reply");
  browser("find", "role", "button", "click", "--name", "React with 🎉", "--exact");
  wait(
    `document.querySelector('${thread(broadcast.clientMessageId)} .chat-reaction')?.getAttribute('aria-pressed') === 'true' && document.querySelector('${channel(broadcast.clientMessageId)} .chat-reaction')?.getAttribute('aria-pressed') === 'true'`,
  );
  browser("focus", "#chat-thread-reply");
  browser("mouse", "move", "1400", "30");
  screenshot("threads-desktop");
  checkThreadResize(root.clientMessageId);
  browser("fill", "#chat-thread-reply", "Saved draft for Alex");
  const requestCount = evaluate("window.threadRequests.length");
  const positions = () =>
    evaluate(
      `Array.from(document.querySelectorAll('.chat-thread-panel .chat-message')).map(row => row.getBoundingClientRect().top)`,
    );
  const before = positions();
  browser("click", `${channel(root.clientMessageId)} .chat-thread-summary`);
  browser("click", `${channel(root.clientMessageId)} .chat-thread-summary`);
  assert.equal(evaluate("window.threadRequests.length"), requestCount, "repeated clicks cannot fetch again");
  assert.deepEqual(positions(), before, "repeated clicks cannot shift the displayed replies");
  assert.equal(
    evaluate('!!document.querySelector(".chat-thread-skeleton, .chat-thread-messages[aria-busy=true]")'),
    false,
  );
  assert.equal(evaluate('document.querySelector("#chat-thread-reply").value'), "Saved draft for Alex");
  if (process.env.MESSAGE_TEST_CHROME) {
    browser("hover", `${channel(other.clientMessageId)} .chat-reply-thread`);
    wait(`window.threadRequests.some(url => url.includes('/${other.id}/thread'))`);
    assert.equal(
      evaluate(`!!document.querySelector('${thread(root.clientMessageId)}')`),
      true,
      "hover does not change the open thread",
    );
  }
  evaluate(`(() => {
    window.pinsChannel = document.querySelector('.chat-scroller');
    window.pinsThread = document.querySelector('.chat-thread-panel');
    window.pinsScrollTop = pinsChannel.scrollTop;
  })()`);
  browser("click", ".chat-pins-toggle");
  wait('!!document.querySelector(".chat-pins-dialog")');
  evaluate(`(() => {
    window.threadPinFrames = [];
    const sample = () => {
      if (!document.querySelector('.chat-pins-dialog')) {
        const scroller = document.querySelector('.chat-scroller');
        const bounds = scroller.getBoundingClientRect();
        threadPinFrames.push(scroller === pinsChannel && document.querySelector('.chat-thread-panel') === pinsThread
          && Math.abs(scroller.scrollTop - pinsScrollTop) < 2
          && [...scroller.querySelectorAll('.chat-message')].some(row => row.checkVisibility({ visibilityProperty: true, opacityProperty: true })
            && row.getBoundingClientRect().bottom > bounds.top && row.getBoundingClientRect().top < bounds.bottom));
      }
      if (threadPinFrames.length < 30) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  })()`);
  browser("find", "role", "button", "click", "--name", "Close pins", "--exact");
  wait("threadPinFrames.length === 30");
  assert.equal(
    evaluate("threadPinFrames.every(Boolean)"),
    true,
    "Pins return with an open thread must not flash or move",
  );
  assert.equal(evaluate('document.querySelector("#chat-thread-reply").value'), "Saved draft for Alex");
  screenshot("threads-pins-return");
  open(other.clientMessageId);
  assert.equal(evaluate('document.querySelector("#chat-thread-reply").value'), "");
  browser("fill", "#chat-thread-reply", "Saved draft for Maya");
  open(root.clientMessageId);
  assert.equal(evaluate('document.querySelector("#chat-thread-reply").value'), "Saved draft for Alex");
  close();
  assert.equal(
    evaluate(
      `document.activeElement === document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread')`,
    ),
    true,
    "Closing restores trigger focus",
  );
  browser("click", `${channel(broadcast.clientMessageId)} .chat-thread-context`);
  wait(`!!document.querySelector('${thread(root.clientMessageId)}')`);
  assert.equal(
    evaluate(`window.threadRequests.filter(url => url.includes('/${root.id}/thread')).length`),
    1,
    "closing and reopening uses the loaded thread",
  );
  assert.equal(
    evaluate('document.querySelectorAll(".chat-thread-panel .chat-message").length'),
    3,
    "A broadcast opens its original root, never a nested thread",
  );
  close();
  await checkReplyDelivery(root, other, "desktop");

  // Explicitly labelled response mock exercises loading/error/retry without an external outage.
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.threadFetch = original;
    window.fetch = (input, init) => String(input).includes('/thread') ? new Promise(resolve => { window.releaseThread = () => resolve(Response.json({ error: 'TEST FIXTURE: thread unavailable' }, { status: 503 })); }) : original(input, init);
  })()`);
  browser("click", `${channel(initial.messages[0].clientMessageId)} .chat-reply-thread`);
  wait('document.querySelector(".chat-thread-messages").getAttribute("aria-busy") === "true"');
  assert.equal(
    evaluate('document.querySelector(".chat-thread-skeleton").getAttribute("aria-label")'),
    "Loading thread replies",
  );
  assert.equal(evaluate('document.querySelector(".chat-thread-status")'), null, "no text loading row");
  alignedHeaders(1440);
  screenshot("threads-desktop-loading");
  evaluate("window.releaseThread()");
  wait('document.querySelector(".chat-thread-status[role=alert]")?.textContent.includes("TEST FIXTURE")');
  alignedHeaders(1440);
  screenshot("threads-desktop-error");
  evaluate("window.fetch = window.threadFetch");
  browser("click", ".chat-thread-status[role=alert] button");
  wait(
    '!document.querySelector(".chat-thread-status[role=alert]") && !document.querySelector(".chat-thread-messages[aria-busy=true]")',
  );
  close();

  browser("find", "role", "button", "click", "--name", "Show member list", "--exact");
  wait('!!document.querySelector("#space-member-list")');
  browser("set", "viewport", "390", "844", "2");
  // Narrowing dismisses the desktop member column instead of covering chat.
  wait('!document.querySelector("#space-member-list")');
  socket = new WebSocket(browser("get", "cdp-url").cdpUrl);
  await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
  let nextID = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data),
      request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    if (message.error) request.reject(new Error(message.error.message));
    else request.resolve(message.result);
  });
  const cdp = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      const id = ++nextID;
      pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params, sessionId }));
    });
  const { targetInfos } = await cdp("Target.getTargets");
  const target = targetInfos.find((target) => target.type === "page" && target.url.startsWith(web));
  const { sessionId } = await cdp("Target.attachToTarget", { targetId: target.targetId, flatten: true });
  await cdp("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 }, sessionId);
  assert.equal(evaluate('matchMedia("(pointer: coarse)").matches'), true);
  const checkKeyboardDismiss = async (scroller, composer) => {
    const saved = evaluate(`document.querySelector('${composer}').value`);
    const top = evaluate(`document.querySelector('${scroller}').scrollTop`);
    const draft = "TEST FIXTURE — scrolling keeps this unsent draft";
    browser("fill", composer, draft);
    evaluate(`(() => {
      const list = document.querySelector('${scroller}');
      list.scrollTop = list.scrollHeight;
      list.dispatchEvent(new Event('scroll'));
    })()`);
    assert.equal(evaluate("document.activeElement.id"), composer.slice(1), "Automatic scrolling keeps focus");
    const point = evaluate(`(() => {
      const r = document.querySelector('${scroller}').getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + Math.min(r.height / 2, 180) };
    })()`);
    const touch = (type, dx = 0, dy = 0) =>
      cdp(
        "Input.dispatchTouchEvent",
        {
          type,
          touchPoints: type === "touchEnd" || type === "touchCancel" ? [] : [{ x: point.x + dx, y: point.y + dy }],
        },
        sessionId,
      );
    // A small movement/cancel is not a scroll gesture and must not dismiss.
    await touch("touchStart");
    await touch("touchMove", 2, 5);
    assert.equal(evaluate("document.activeElement.id"), composer.slice(1));
    await touch("touchCancel");
    if (composer === "#chat-thread-reply") {
      // Threads have no horizontal Browse navigation to legitimately dismiss focus.
      await touch("touchStart");
      await touch("touchMove", 60, 2);
      assert.equal(evaluate("document.activeElement.id"), composer.slice(1), "Horizontal movement keeps focus");
      await touch("touchCancel");
    }
    for (const dy of [120, -120]) {
      browser("focus", composer);
      await touch("touchStart");
      await touch("touchMove", 3, dy);
      assert.notEqual(evaluate("document.activeElement.id"), composer.slice(1), "Timeline dragging dismisses focus");
      await touch("touchEnd");
      assert.equal(evaluate(`document.querySelector('${composer}').value`), draft, "Dismissal retains the draft");
    }
    const multiline = Array.from({ length: 20 }, (_, index) => `Unsent line ${index}`).join("\n");
    browser("fill", composer, multiline);
    const inputPoint = evaluate(`(() => {
      const r = document.querySelector('${composer}').getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 3 };
    })()`);
    await cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [inputPoint] }, sessionId);
    await cdp(
      "Input.dispatchTouchEvent",
      { type: "touchMove", touchPoints: [{ x: inputPoint.x, y: inputPoint.y + 40 }] },
      sessionId,
    );
    assert.equal(evaluate("document.activeElement.id"), composer.slice(1), "Scrolling inside the draft keeps focus");
    assert.equal(evaluate(`document.querySelector('${composer}').value`), multiline);
    await cdp("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] }, sessionId);
    browser("focus", composer);
    assert.equal(evaluate("document.activeElement.id"), composer.slice(1), "The composer can reopen normally");
    browser("fill", composer, saved);
    evaluate(`document.querySelector('${composer}').blur()`);
    evaluate(`document.querySelector('${scroller}').scrollTop = ${top}`);
    console.log(
      `PASS: ${composer}: vertical touch dismissal in both directions, retained draft, focus reopening, automatic-scroll and cancelled-gesture controls.`,
    );
  };
  await checkKeyboardDismiss(".chat-scroller", "#chat-message");
  assert.equal(
    evaluate(
      `(() => { const row = document.querySelector('${channel(root.clientMessageId)}'); const reply = row.querySelector('.chat-reply-thread'); const summary = row.querySelector('.chat-thread-summary').getBoundingClientRect(); const more = row.querySelector('.chat-message-actions-trigger').getBoundingClientRect(); return !reply.checkVisibility({ visibilityProperty: true }) && summary.width > 0 && summary.height > 0 && more.width >= 44 && more.height >= 44 && (summary.right <= more.left || summary.top >= more.bottom); })()`,
    ),
    true,
    "Touch layout hides inline replies and keeps thread links separate from 44px message actions",
  );
  screenshot("threads-mobile-channel");
  const summarySelector = `${channel(root.clientMessageId)} .chat-thread-summary`;
  const summaryPoint = evaluate(`(() => {
    const r = document.querySelector('${summarySelector}').getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  })()`);
  await cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [summaryPoint] }, sessionId);
  await new Promise((resolve) => setTimeout(resolve, 700));
  assert.notEqual(
    evaluate(`getComputedStyle(document.querySelector('${summarySelector}')).backgroundColor`),
    "rgba(0, 0, 0, 0)",
    "Thread feedback remains visible throughout a long press",
  );
  assert.equal(evaluate('!!document.querySelector(".chat-message-actions")'), false);
  await cdp("Input.dispatchTouchEvent", { type: "touchCancel", touchPoints: [] }, sessionId);
  assert.equal(evaluate(`document.querySelector('${summarySelector}').hasAttribute('data-pressing')`), false);
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.threadFetch = original;
    window.fetch = (input, init) => String(input).includes('/thread') ? new Promise((resolve, reject) => { window.releaseThread = () => original(input, init).then(resolve, reject); }) : original(input, init);
  })()`);
  actions(channel(initial.messages[2].clientMessageId), true);
  browser("find", "role", "button", "click", "--name", "Reply in thread", "--exact");
  wait('!!document.querySelector(".chat-thread-skeleton")');
  screenshot("threads-mobile-loading");
  evaluate("window.fetch = window.threadFetch; window.releaseThread()");
  wait('!document.querySelector(".chat-thread-messages[aria-busy=true]")');
  close();
  actions(channel(root.clientMessageId), true);
  dismissActions();
  open(root.clientMessageId);
  wait('document.querySelector(".chat-thread-panel").getAttribute("aria-modal") === "true"');
  assert.equal(evaluate('document.querySelector(".chat-panel").inert'), true);
  assert.deepEqual(
    evaluate(
      '(() => { const rect = document.querySelector(".chat-thread-panel").getBoundingClientRect(); return [rect.x, rect.y, rect.width, rect.height]; })()',
    ),
    // Classic-scrollbar Chromium reserves the root's stable gutter; a phone
    // with overlay scrollbars has the full 390px layout viewport instead.
    [0, 0, evaluate("document.documentElement.getBoundingClientRect().width"), 844],
  );
  alignedHeaders(390);
  assert.equal(evaluate('document.querySelector(".chat-thread-resize").checkVisibility()'), false);
  await checkKeyboardDismiss(".chat-thread-messages", "#chat-thread-reply");
  const swipeThread = async (selector, dx, dy = 0, cancel = false) => {
    const point = evaluate(`(() => {
      const r = document.querySelector('${selector}').getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    })()`);
    await cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point] }, sessionId);
    for (let step = 1; step <= 8; step++) {
      await cdp(
        "Input.dispatchTouchEvent",
        {
          type: "touchMove",
          touchPoints: [{ x: point.x + (dx * step) / 8, y: point.y + (dy * step) / 8 }],
        },
        sessionId,
      );
    }
    await cdp("Input.dispatchTouchEvent", { type: cancel ? "touchCancel" : "touchEnd", touchPoints: [] }, sessionId);
  };
  const savedThreadDraft = evaluate('document.querySelector("#chat-thread-reply").value');
  for (const [dx, dy, cancel] of [
    [-140, 0, false],
    [50, 0, false],
    [140, 0, true],
    [4, 120, false],
  ]) {
    await swipeThread(".chat-thread-messages", dx, dy, cancel);
    assert.equal(
      evaluate('!!document.querySelector(".chat-thread-panel")'),
      true,
      `Left, short, cancelled and vertical drags keep the thread: ${dx}, ${dy}, ${cancel}`,
    );
  }
  await swipeThread("#chat-thread-reply", 90);
  assert.equal(evaluate('!!document.querySelector(".chat-thread-panel")'), true, "The composer keeps its own gestures");
  assert.equal(evaluate('document.querySelector("#chat-thread-reply").value'), savedThreadDraft);
  await swipeThread(".chat-thread-messages", 140);
  wait('!document.querySelector(".chat-thread-panel") && !history.state?.caperThread');
  assert.equal(evaluate('document.querySelector(".chat-panel").inert'), false);
  assert.equal(evaluate('!!document.querySelector("#chat-heading")'), true, "Swipe returns to the channel");
  open(root.clientMessageId);
  assert.equal(
    evaluate('document.querySelector("#chat-thread-reply").value'),
    savedThreadDraft,
    "Swipe keeps the unsent draft",
  );
  await swipeThread(".chat-thread-heading", 140);
  wait('!document.querySelector(".chat-thread-panel") && !history.state?.caperThread');
  open(root.clientMessageId);
  console.log(
    "PASS: thread right-swipes from timeline/header return to channel and preserve draft; left/short/cancelled/vertical/composer drags stay in thread.",
  );
  screenshot("threads-mobile");
  evaluate('document.querySelector(".chat-thread-heading span").textContent = "in #" + "long-channel-name-".repeat(3)');
  browser("set", "viewport", "320", "844", "2");
  alignedHeaders(320);
  screenshot("threads-mobile-long-name");
  browser("set", "viewport", "390", "844", "2");
  evaluate(`document.querySelector(".chat-thread-heading span").textContent = ${JSON.stringify(originalName)}`);
  actions(thread(root.clientMessageId), false);
  dismissActions();
  // Exercise the held-finger path as well as the overflow button.
  const point = evaluate(
    `(() => { const rect = document.querySelector('${thread(broadcast.clientMessageId)} p').getBoundingClientRect(); return { x: rect.x + 10, y: rect.y + 10 }; })()`,
  );
  await cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point] }, sessionId);
  const held = await cdp(
    "Runtime.evaluate",
    {
      expression: `new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve({
      thread: document.querySelector('${thread(broadcast.clientMessageId)}').dataset.pressing,
      channel: document.querySelector('${channel(broadcast.clientMessageId)}')?.dataset.pressing ?? null,
      wash: getComputedStyle(document.querySelector('${thread(broadcast.clientMessageId)}')).backgroundImage,
      menuOpen: !!document.querySelector('.chat-message-actions'),
    }))))`,
      awaitPromise: true,
      returnByValue: true,
    },
    sessionId,
  );
  assert.equal(held.result.value.thread, "true", "The held reply must highlight before its menu opens");
  assert.equal(held.result.value.channel, null, "The broadcast copy in the channel must not highlight");
  assert.notEqual(held.result.value.wash, "none");
  assert.equal(held.result.value.menuOpen, false);
  await new Promise((resolve) => setTimeout(resolve, 650));
  await cdp("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] }, sessionId);
  assertReplyAction(false);
  screenshot("thread-actions-mobile-reply");
  browser("find", "role", "button", "click", "--name", "Reply", "--exact");
  wait(
    '!document.querySelector(".chat-message-actions") && !!document.querySelector(".chat-thread-panel .chat-reply-draft")',
  );
  assertReplyDraft(".chat-thread-panel", broadcast);
  screenshot("reply-draft-thread-mobile");
  assert.equal(evaluate("document.activeElement.id"), "chat-thread-reply");
  browser("find", "role", "button", "click", "--name", "Cancel reply", "--exact");
  evaluate("history.back()");
  wait('!document.querySelector(".chat-thread-panel")');
  assert.equal(evaluate('document.querySelector(".chat-panel").inert'), false);
  open(root.clientMessageId);
  browser("click", ".chat-thread-back");
  wait('!document.querySelector(".chat-thread-panel")');
  assert.equal(evaluate('document.querySelector(".chat-panel").inert'), false);
  await checkReplyDelivery(root, other, "mobile");
  browser("set", "viewport", "1440", "900", "2");
  browser("open", `${web}/spaces`);
  wait(
    `!!document.querySelector('${channel(root.clientMessageId)}') && !document.querySelector('.chat-initial-messages')`,
  );
  open(root.clientMessageId);
  assert.equal(
    evaluate('document.querySelector(".chat-thread-panel").getBoundingClientRect().width'),
    423,
    "Preferred width survives a page reload and mobile layout",
  );
  await checkReactionFollow(root, "desktop");
  browser("set", "viewport", "390", "844", "2");
  evaluate("document.querySelector('.chat-thread-messages').scrollTop = 1e9");
  await checkReactionFollow(root, "narrow");
  close();
  await cdp("Emulation.setTouchEmulationEnabled", { enabled: false }, sessionId);
  browser("set", "viewport", "1440", "900", "2");
  await checkLongQuotePreview("desktop");
  await cdp("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 }, sessionId);
  browser("set", "viewport", "390", "844", "2");
  assert.equal(evaluate('matchMedia("(pointer: coarse)").matches'), true);
  await checkLongQuotePreview("mobile");
  console.log(
    "PASS: thread drag/keyboard resizing, bounds, reset, draft retention, saved width after reopen/reload/mobile and window constraints; repeated clicks make zero requests and zero reply-position changes; hover/focus prefetch, cached reopening, cold-load skeletons; aligned 54px headers at 1440/1024/761/390/320px, long-name containment; Reply in thread hidden inside existing threads and retained in the channel; quick Reply selection/focus/cancel, quoted thread send, source navigation and touch long-press Reply; isolated replies, root highlight/count/avatars, shared broadcast/reaction identity, per-root drafts, Pins-return stability, loading/error/retry, quiet in-flight replies, scoped failure recovery, idempotent retry, rejected edit/dismiss, focus restoration, touch target separation, full-screen dialog, inert background and mobile Back.",
  );
} finally {
  socket?.close();
  browser("close");
}
