import assert from "node:assert/strict";
import test from "node:test";

import {
  mergePresenceUpdate,
  parseLivePresenceEvent,
  presenceQueryWantsPubkey,
  PRESENCE_IDLE_TIMEOUT_MS,
  resolveAutomaticPresenceStatus,
} from "./presence.ts";

const WILL = "8e39cba681211b3782d0e4483e9343719b9b7be66515252da5491f26421896b1";
const OTHER =
  "44b8e82baaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

test("merge adds an absent pubkey going online (the core bug)", () => {
  const old = {};
  const next = mergePresenceUpdate(old, WILL, "online");
  assert.deepEqual(next, { [WILL]: "online" });
});

test("merge updates an existing pubkey", () => {
  const next = mergePresenceUpdate({ [WILL]: "offline" }, WILL, "online");
  assert.deepEqual(next, { [WILL]: "online" });
});

test("merge returns same reference when status is unchanged", () => {
  const old = { [WILL]: "online" };
  assert.equal(mergePresenceUpdate(old, WILL, "online"), old);
});

test("merge leaves other pubkeys untouched", () => {
  const next = mergePresenceUpdate({ [OTHER]: "away" }, WILL, "online");
  assert.deepEqual(next, { [OTHER]: "away", [WILL]: "online" });
});

test("merge is a no-op on an undefined cache", () => {
  assert.equal(mergePresenceUpdate(undefined, WILL, "online"), undefined);
});

test("query wants a pubkey it requested", () => {
  assert.equal(presenceQueryWantsPubkey(["presence", WILL, OTHER], WILL), true);
});

test("query does not want a pubkey it did not request", () => {
  assert.equal(presenceQueryWantsPubkey(["presence", OTHER], WILL), false);
});

test("bare presence key (no pubkeys) wants nothing", () => {
  assert.equal(presenceQueryWantsPubkey(["presence"], WILL), false);
});

test("live event keys off the author, not a p tag", () => {
  const event = { pubkey: OTHER, content: "online", tags: [["p", WILL]] };
  assert.deepEqual(parseLivePresenceEvent(event), {
    pubkey: OTHER,
    status: "online",
  });
});

test("spoof attempt cannot mark a victim: foreign p tag is ignored", () => {
  const event = { pubkey: OTHER, content: "offline", tags: [["p", WILL]] };
  const parsed = parseLivePresenceEvent(event);
  assert.notEqual(parsed.pubkey, WILL);
  assert.equal(parsed.pubkey, OTHER);
});

test("live event with unknown status is rejected", () => {
  assert.equal(
    parseLivePresenceEvent({ pubkey: WILL, content: "lurking" }),
    null,
  );
});

test("live event lowercases the author pubkey", () => {
  const parsed = parseLivePresenceEvent({
    pubkey: WILL.toUpperCase(),
    content: "away",
  });
  assert.equal(parsed.pubkey, WILL);
});

test("OS idle below the threshold is online even with stale in-app activity", () => {
  const now = 1_000_000_000;
  const staleActivity = now - 60 * 60_000;
  assert.equal(
    resolveAutomaticPresenceStatus(30, staleActivity, now),
    "online",
  );
});

test("OS idle at the threshold is away even with fresh in-app activity", () => {
  const now = 1_000_000_000;
  assert.equal(resolveAutomaticPresenceStatus(600, now, now), "away");
});

test("without OS idle, fresh in-app activity is online", () => {
  const now = 1_000_000_000;
  assert.equal(
    resolveAutomaticPresenceStatus(null, now - 1_000, now),
    "online",
  );
});

test("without OS idle, in-app inactivity past the threshold is away", () => {
  const now = 1_000_000_000;
  assert.equal(
    resolveAutomaticPresenceStatus(null, now - PRESENCE_IDLE_TIMEOUT_MS, now),
    "away",
  );
});

test("OS idle of zero is online", () => {
  assert.equal(resolveAutomaticPresenceStatus(0, 0, 1_000_000_000), "online");
});
