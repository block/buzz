/**
 * Who a session opens itself for.
 *
 * The rule is narrow by design: the member who asked, of this agent, here, just
 * now. Anything wider lets an agent's announcement seize the screen of whoever
 * happens to be reading the channel — and because the announcement cites its
 * own justification, a citation the agent chose must not be taken at face
 * value. That is why these tests spend most of their effort on refusals.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { AUTO_OPEN_MAX_AGE_SECS, planAutoOpen } from "./agentMediaAutoOpen.ts";

const ME = "a".repeat(64);
const SOMEONE = "b".repeat(64);
const AGENT = "c".repeat(64);
const OTHER_AGENT = "d".repeat(64);
const NOW = 1_700_000_000;

/** A session as the fold would hand it over. */
function session(overrides = {}) {
  return {
    eventId: "e".repeat(64),
    agentPubkey: AGENT,
    channelId: "chan-1",
    sourceEventId: "s".repeat(64),
    startedAt: NOW - 5,
    ...overrides,
  };
}

/** A source message that satisfies every condition unless overridden. */
function source(overrides = {}) {
  return {
    author: ME,
    channelId: "chan-1",
    addressed: new Set([AGENT]),
    ...overrides,
  };
}

const NONE = new Set();
const sourceIs = (value) => () => value;
const unresolved = () => undefined;

test("planAutoOpen opens the session this member asked for", () => {
  const mine = session();
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source()),
    sessions: [mine],
  });
  assert.equal(plan.open, mine);
  assert.deepEqual(plan.resolve, []);
});

test("planAutoOpen opens for a reply that addresses the agent", () => {
  // The reply path p-tags an author as well as any typed mention, so a reply
  // to the agent addresses it without the member typing its name. That is
  // still this member engaging with this agent, and it must not be refused.
  const mine = session();
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source({ addressed: new Set([SOMEONE, AGENT]) })),
    sessions: [mine],
  });
  assert.equal(plan.open, mine);
});

test("planAutoOpen refuses a session somebody else asked for", () => {
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source({ author: SOMEONE })),
    sessions: [session()],
  });
  assert.equal(plan.open, null);
  assert.deepEqual(plan.resolve, []);
});

test("planAutoOpen refuses a source message from another channel", () => {
  // Otherwise a message this member wrote somewhere else — including a channel
  // they are not watching — justifies taking over the view here.
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source({ channelId: "chan-2" })),
    sessions: [session({ channelId: "chan-1" })],
  });
  assert.equal(plan.open, null);
});

test("planAutoOpen refuses a source message with no channel", () => {
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source({ channelId: null })),
    sessions: [session()],
  });
  assert.equal(plan.open, null);
});

test("planAutoOpen refuses a source message that does not address the agent", () => {
  // The load-bearing refusal. Without it every message this member wrote in
  // the channel is a citation the agent can help itself to, and an agent can
  // open its own panel on somebody who never asked it for anything.
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source({ addressed: new Set() })),
    sessions: [session()],
  });
  assert.equal(plan.open, null);
});

test("planAutoOpen refuses a source message addressed to a different agent", () => {
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source({ addressed: new Set([OTHER_AGENT]) })),
    sessions: [session({ agentPubkey: AGENT })],
  });
  assert.equal(plan.open, null);
});

test("planAutoOpen asks for an unknown source instead of assuming", () => {
  // An unresolved lookup read as "not mine" would mean the panel never opens
  // and nothing ever says why.
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: unresolved,
    sessions: [session()],
  });
  assert.equal(plan.open, null);
  assert.deepEqual(plan.resolve, ["s".repeat(64)]);
});

test("planAutoOpen does not retry a lookup that found nothing", () => {
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs({ author: "", channelId: null, addressed: new Set() }),
    sessions: [session()],
  });
  assert.equal(plan.open, null);
  assert.deepEqual(
    plan.resolve,
    [],
    "an unreadable source is resolved, not pending",
  );
});

test("planAutoOpen refuses a session it has already handled", () => {
  // Closing the panel has to be final, or auto-open becomes an argument.
  const mine = session();
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: new Set([mine.eventId]),
    nowSeconds: NOW,
    sourceOf: sourceIs(source()),
    sessions: [mine],
  });
  assert.equal(plan.open, null);
});

test("planAutoOpen refuses a session older than the window", () => {
  // Re-entering a channel is not a fresh request.
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source()),
    sessions: [session({ startedAt: NOW - AUTO_OPEN_MAX_AGE_SECS - 1 })],
  });
  assert.equal(plan.open, null);
  assert.deepEqual(plan.resolve, [], "a stale session is not worth resolving");
});

test("planAutoOpen accepts a session right at the age limit", () => {
  const mine = session({ startedAt: NOW - AUTO_OPEN_MAX_AGE_SECS });
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source()),
    sessions: [mine],
  });
  assert.equal(plan.open, mine);
});

test("planAutoOpen refuses a session that names no source message", () => {
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source()),
    sessions: [session({ sourceEventId: null })],
  });
  assert.equal(plan.open, null);
});

test("planAutoOpen does nothing before this member's identity is known", () => {
  const plan = planAutoOpen({
    currentPubkey: null,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: sourceIs(source()),
    sessions: [session()],
  });
  assert.equal(plan.open, null);
  assert.deepEqual(plan.resolve, []);
});

test("planAutoOpen takes the newest decidable session", () => {
  // A newer session still waiting on a lookup must not hold back an older one
  // that already qualifies.
  const pending = session({
    eventId: "1".repeat(64),
    sourceEventId: "9".repeat(64),
    startedAt: NOW - 1,
  });
  const mine = session({ eventId: "2".repeat(64), startedAt: NOW - 10 });
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: (id) => (id === mine.sourceEventId ? source() : undefined),
    sessions: [pending, mine],
  });
  assert.equal(plan.open, mine);
});

test("planAutoOpen keeps looking after refusing a decidable session", () => {
  // A refusal is not a stop. An agent that cites an unusable message must not
  // shadow a session further down the list that this member really did ask for.
  const theirs = session({
    eventId: "1".repeat(64),
    sourceEventId: "8".repeat(64),
    startedAt: NOW - 1,
  });
  const mine = session({ eventId: "2".repeat(64), startedAt: NOW - 10 });
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    sourceOf: (id) =>
      id === mine.sourceEventId ? source() : source({ author: SOMEONE }),
    sessions: [theirs, mine],
  });
  assert.equal(plan.open, mine);
});
