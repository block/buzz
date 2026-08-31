/**
 * Who a session opens itself for.
 *
 * The rule is narrow by design: the member who asked. Anything wider lets an
 * agent's announcement seize the screen of whoever happens to be reading the
 * channel, which is why these tests spend most of their effort on refusals.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { AUTO_OPEN_MAX_AGE_SECS, planAutoOpen } from "./agentMediaAutoOpen.ts";

const ME = "a".repeat(64);
const SOMEONE = "b".repeat(64);
const NOW = 1_700_000_000;

/** A session as the fold would hand it over. */
function session(overrides = {}) {
  return {
    eventId: "e".repeat(64),
    agentPubkey: "c".repeat(64),
    channelId: "chan-1",
    sourceEventId: "s".repeat(64),
    startedAt: NOW - 5,
    ...overrides,
  };
}

const NONE = new Set();
const requesterIs = (pubkey) => () => pubkey;
const unresolved = () => undefined;

test("planAutoOpen opens the session this member asked for", () => {
  const mine = session();
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    requesterOf: requesterIs(ME),
    sessions: [mine],
  });
  assert.equal(plan.open, mine);
  assert.deepEqual(plan.resolve, []);
});

test("planAutoOpen refuses a session somebody else asked for", () => {
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    requesterOf: requesterIs(SOMEONE),
    sessions: [session()],
  });
  assert.equal(plan.open, null);
  assert.deepEqual(plan.resolve, []);
});

test("planAutoOpen asks for an unknown author instead of assuming", () => {
  // An unresolved lookup read as "not mine" would mean the panel never opens
  // and nothing ever says why.
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    requesterOf: unresolved,
    sessions: [session()],
  });
  assert.equal(plan.open, null);
  assert.deepEqual(plan.resolve, ["s".repeat(64)]);
});

test("planAutoOpen does not retry a lookup that found no author", () => {
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    requesterOf: requesterIs(""),
    sessions: [session()],
  });
  assert.equal(plan.open, null);
  assert.deepEqual(
    plan.resolve,
    [],
    "an empty author is resolved, not pending",
  );
});

test("planAutoOpen refuses a session it has already handled", () => {
  // Closing the panel has to be final, or auto-open becomes an argument.
  const mine = session();
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: new Set([mine.eventId]),
    nowSeconds: NOW,
    requesterOf: requesterIs(ME),
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
    requesterOf: requesterIs(ME),
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
    requesterOf: requesterIs(ME),
    sessions: [mine],
  });
  assert.equal(plan.open, mine);
});

test("planAutoOpen refuses a session that names no source message", () => {
  const plan = planAutoOpen({
    currentPubkey: ME,
    handled: NONE,
    nowSeconds: NOW,
    requesterOf: requesterIs(ME),
    sessions: [session({ sourceEventId: null })],
  });
  assert.equal(plan.open, null);
});

test("planAutoOpen does nothing before this member's identity is known", () => {
  const plan = planAutoOpen({
    currentPubkey: null,
    handled: NONE,
    nowSeconds: NOW,
    requesterOf: requesterIs(ME),
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
    requesterOf: (id) => (id === mine.sourceEventId ? ME : undefined),
    sessions: [pending, mine],
  });
  assert.equal(plan.open, mine);
});
