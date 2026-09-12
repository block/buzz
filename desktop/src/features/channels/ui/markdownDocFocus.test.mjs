/**
 * Opener-identity tests for the markdown panel focus restore (PR #6731 P2
 * follow-up). The same attachment can appear in several messages — several
 * cards sharing one `data-doc-url` — so restoring by URL alone always lands
 * on the first DOM match. These tests pin the recorded per-invocation
 * identity: the invoking card wins, the record is consumed after one
 * restore, a missing record falls back to the first match, and a claimed
 * focus target aborts the restore entirely.
 */

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

// pretendToBeVisual gives the module its requestAnimationFrame loop.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    // This jsdom build has no window.CSS; quote-safe escaping is all the
    // module's attribute selectors need.
    CSS: { escape: (value) => String(value).replace(/["\\]/g, "\\$&") },
    cancelAnimationFrame: dom.window.cancelAnimationFrame.bind(dom.window),
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
    window: dom.window,
  });
});

after(() => dom.window.close());

const DOC_URL = "http://localhost:3000/media/deadbeef.bin";

let nextMessageId = 0;
function addCard(url = DOC_URL, messageId = `message-${nextMessageId++}`) {
  const row = dom.window.document.createElement("article");
  row.setAttribute("data-testid", "message-row");
  row.setAttribute("data-message-id", messageId);
  const card = dom.window.document.createElement("button");
  card.setAttribute("data-testid", "file-card");
  card.setAttribute("data-doc-url", url);
  row.appendChild(card);
  dom.window.document.body.appendChild(row);
  return card;
}

function settleFrames(count = 3) {
  let done = Promise.resolve();
  for (let i = 0; i < count; i += 1) {
    done = done.then(
      () => new Promise((resolve) => dom.window.requestAnimationFrame(resolve)),
    );
  }
  return done;
}

async function loadModule() {
  return import("./markdownDocFocus.ts");
}

beforeEach(async () => {
  dom.window.document.body.innerHTML = "";
  // Park focus on <body> (focus is "free") and clear any leftover record.
  const { recordMarkdownDocOpener } = await loadModule();
  recordMarkdownDocOpener(DOC_URL, null);
});

test("restores the recorded invoking card, not the first URL match", async () => {
  const { recordMarkdownDocOpener, restoreFocusToMarkdownDocOpener } =
    await loadModule();
  addCard();
  const second = addCard();

  recordMarkdownDocOpener(DOC_URL, second);
  restoreFocusToMarkdownDocOpener(DOC_URL);
  await settleFrames();

  assert.equal(dom.window.document.activeElement, second);
});

test("consumes the record: a later restore without one takes the first match", async () => {
  const { recordMarkdownDocOpener, restoreFocusToMarkdownDocOpener } =
    await loadModule();
  const first = addCard();
  const second = addCard();

  recordMarkdownDocOpener(DOC_URL, second);
  restoreFocusToMarkdownDocOpener(DOC_URL);
  await settleFrames();
  assert.equal(dom.window.document.activeElement, second);

  // Free the focus again, then restore with no fresh record (deep link /
  // reload open): the stale index must not survive the first consumption.
  second.blur();
  restoreFocusToMarkdownDocOpener(DOC_URL);
  await settleFrames();
  assert.equal(dom.window.document.activeElement, first);
});

test("tracks the opener by message id when a preceding same-URL card disappears", async () => {
  const { recordMarkdownDocOpener, restoreFocusToMarkdownDocOpener } =
    await loadModule();
  const first = addCard(DOC_URL, "message-a");
  const opener = addCard(DOC_URL, "message-b");
  const following = addCard(DOC_URL, "message-c");

  recordMarkdownDocOpener(DOC_URL, opener);
  first.closest('[data-testid="message-row"]').remove();
  restoreFocusToMarkdownDocOpener(DOC_URL);
  await settleFrames();

  assert.equal(dom.window.document.activeElement, opener);
  assert.notEqual(dom.window.document.activeElement, following);
});

test("falls back to a surviving same-URL card when the opener is gone", async () => {
  const { recordMarkdownDocOpener, restoreFocusToMarkdownDocOpener } =
    await loadModule();
  const first = addCard();
  const second = addCard();

  recordMarkdownDocOpener(DOC_URL, second);
  second.remove();
  restoreFocusToMarkdownDocOpener(DOC_URL);
  await settleFrames();

  assert.equal(dom.window.document.activeElement, first);
});

test("restores a thread-only opener to its surviving summary control", async () => {
  const { recordMarkdownDocOpener, restoreFocusToMarkdownDocOpener } =
    await loadModule();
  const threadPanel = dom.window.document.createElement("section");
  threadPanel.setAttribute("data-testid", "message-thread-panel");
  dom.window.document.body.appendChild(threadPanel);
  const head = addCard(DOC_URL, "thread-head").closest(
    '[data-testid="message-row"]',
  );
  const reply = addCard(DOC_URL, "thread-reply").closest(
    '[data-testid="message-row"]',
  );
  threadPanel.append(head, reply);
  const opener = reply.querySelector('[data-testid="file-card"]');
  const summary = dom.window.document.createElement("button");
  summary.setAttribute("data-testid", "message-thread-summary");
  summary.setAttribute("data-thread-head-id", "thread-head");
  dom.window.document.body.appendChild(summary);

  recordMarkdownDocOpener(DOC_URL, opener);
  threadPanel.remove();
  restoreFocusToMarkdownDocOpener(DOC_URL);
  await settleFrames();

  assert.equal(dom.window.document.activeElement, summary);
});

test("ignores a record made for a different document URL", async () => {
  const { recordMarkdownDocOpener, restoreFocusToMarkdownDocOpener } =
    await loadModule();
  const first = addCard();
  addCard();
  const otherCard = addCard("http://localhost:3000/media/cafe.bin");

  recordMarkdownDocOpener("http://localhost:3000/media/cafe.bin", otherCard);
  restoreFocusToMarkdownDocOpener(DOC_URL);
  await settleFrames();

  assert.equal(dom.window.document.activeElement, first);
});

test("aborts when another control already claimed focus", async () => {
  const { recordMarkdownDocOpener, restoreFocusToMarkdownDocOpener } =
    await loadModule();
  addCard();
  const second = addCard();
  const claimed = dom.window.document.createElement("button");
  dom.window.document.body.appendChild(claimed);

  recordMarkdownDocOpener(DOC_URL, second);
  claimed.focus();
  restoreFocusToMarkdownDocOpener(DOC_URL);
  await settleFrames();

  assert.equal(dom.window.document.activeElement, claimed);
});
