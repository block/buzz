import assert from "node:assert/strict";
import test from "node:test";

import {
  acquireHeadFetchTicket,
  isCurrentHeadFetch,
  recordHeadFetchPublication,
  registerSubscriptionIntent,
  resetChannelOpenGateForTests,
} from "./channelOpenGate.ts";

const CHAN = "chan-1";

function pending(promise) {
  const state = { settled: false, value: undefined };
  promise.then((value) => {
    state.settled = true;
    state.value = value;
  });
  return state;
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

test.beforeEach(() => resetChannelOpenGateForTests());

test("no intent: ticket resolves immediately, unordered", async () => {
  const ticket = await acquireHeadFetchTicket(CHAN);
  assert.equal(ticket.ordered, false);
});

test("intent pending: fetch waits, activation releases it ordered", async () => {
  const intent = registerSubscriptionIntent(CHAN);
  const state = pending(acquireHeadFetchTicket(CHAN));
  await tick();
  assert.equal(state.settled, false, "fetch must wait for activation");
  const needsRefetch = intent.activate();
  await tick();
  assert.equal(state.settled, true);
  assert.equal(state.value.ordered, true);
  assert.equal(
    needsRefetch,
    false,
    "a gated fetch will run ordered — no refetch needed",
  );
});

test("subscription already active: ticket immediate and ordered", async () => {
  const intent = registerSubscriptionIntent(CHAN);
  intent.activate();
  const ticket = await acquireHeadFetchTicket(CHAN);
  assert.equal(ticket.ordered, true);
});

test("activation coverage: a new epoch treats prior-session publications as dead", async () => {
  const intent = registerSubscriptionIntent(CHAN);
  const ticketPromise = acquireHeadFetchTicket(CHAN);
  intent.activate();
  const ticket = await ticketPromise;
  recordHeadFetchPublication(CHAN, ticket);
  const intent2 = registerSubscriptionIntent(CHAN);
  assert.equal(
    intent2.activate(),
    true,
    "new epoch: prior ordered publication is from a dead session — refetch",
  );
});

test("remount (channel A→B→A): stale-epoch publication fails coverage", async () => {
  // Session 1: ordered fetch publishes.
  const intent1 = registerSubscriptionIntent(CHAN);
  const p1 = acquireHeadFetchTicket(CHAN);
  intent1.activate();
  const t1 = await p1;
  recordHeadFetchPublication(CHAN, t1);
  intent1.dispose();
  // Session 2 (remount): no new fetch ran; coverage must fail.
  const intent2 = registerSubscriptionIntent(CHAN);
  assert.equal(intent2.activate(), true);
});

test("unordered prefetch publication never satisfies coverage", async () => {
  const ticket = await acquireHeadFetchTicket(CHAN); // no intent: unordered
  recordHeadFetchPublication(CHAN, ticket);
  const intent = registerSubscriptionIntent(CHAN);
  assert.equal(
    intent.activate(),
    true,
    "unordered publication cannot prove the gap is covered",
  );
});

test("generation gating: superseded fetch loses publication rights", async () => {
  const stale = await acquireHeadFetchTicket(CHAN);
  const fresh = await acquireHeadFetchTicket(CHAN);
  assert.equal(isCurrentHeadFetch(CHAN, stale.generation), false);
  assert.equal(isCurrentHeadFetch(CHAN, fresh.generation), true);
  // A stale publication record is ignored.
  recordHeadFetchPublication(CHAN, stale);
  const intent = registerSubscriptionIntent(CHAN);
  assert.equal(intent.activate(), true, "stale record must not cover");
});

test("adversarial ordering: unordered in-flight, activation, ordered publishes, stale resolves last", async () => {
  // 1. Unordered prefetch starts (no intent).
  const staleTicket = await acquireHeadFetchTicket(CHAN);
  // 2. Subscription activates.
  const intent = registerSubscriptionIntent(CHAN);
  const needsRefetch = intent.activate();
  assert.equal(needsRefetch, true);
  // 3. Ordered fetch starts and publishes.
  const orderedTicket = await acquireHeadFetchTicket(CHAN);
  assert.equal(orderedTicket.ordered, true);
  assert.equal(isCurrentHeadFetch(CHAN, orderedTicket.generation), true);
  recordHeadFetchPublication(CHAN, orderedTicket);
  // 4. Unordered transport resolves LAST: publication gate must refuse it.
  assert.equal(
    isCurrentHeadFetch(CHAN, staleTicket.generation),
    false,
    "stale unordered completion must not publish over the ordered result",
  );
});

test("intent dispose releases a gated fetch unordered (fail open)", async () => {
  const intent = registerSubscriptionIntent(CHAN);
  const state = pending(acquireHeadFetchTicket(CHAN));
  await tick();
  assert.equal(state.settled, false);
  intent.dispose();
  await tick();
  assert.equal(state.settled, true);
  assert.equal(state.value.ordered, false);
});

test("gate timeout releases the fetch unordered", async () => {
  registerSubscriptionIntent(CHAN);
  const ticket = await acquireHeadFetchTicket(CHAN, 10);
  assert.equal(ticket.ordered, false);
});

test("newer intent supersedes an older one's activate/dispose", async () => {
  const stale = registerSubscriptionIntent(CHAN);
  const fresh = registerSubscriptionIntent(CHAN);
  const state = pending(acquireHeadFetchTicket(CHAN));
  stale.dispose(); // must NOT release the fetch — epoch superseded
  await tick();
  assert.equal(state.settled, false, "stale dispose must be inert");
  assert.equal(stale.activate(), false, "stale activate must be inert");
  fresh.activate();
  await tick();
  assert.equal(state.settled, true);
  assert.equal(state.value.ordered, true);
});

test("per-channel isolation", async () => {
  registerSubscriptionIntent("chan-A");
  const ticket = await acquireHeadFetchTicket("chan-B");
  assert.equal(ticket.ordered, false, "chan-B has no intent — immediate");
});
