import assert from "node:assert/strict";
import test from "node:test";

import {
  HERE_FRESHNESS_WINDOW_SECONDS,
  channelNotifyEscalates,
  notifyModeForTags,
} from "./channelNotifyEscalation.ts";

const PUBKEY = "a".repeat(64);
const AUTHOR = "b".repeat(64);
const NOW = 1_700_000_000;

function makeEvent(tags = [], overrides = {}) {
  return {
    id: `event-${"0".repeat(59)}`,
    pubkey: AUTHOR,
    created_at: NOW,
    kind: 9,
    tags,
    content: "hello",
    sig: "s".repeat(128),
    ...overrides,
  };
}

const LIVE = { selfOnline: true, nowSeconds: NOW };

test("notifyModeForTags: reads channel and here", () => {
  assert.equal(notifyModeForTags([["notify", "channel"]]), "channel");
  assert.equal(
    notifyModeForTags([
      ["h", "c"],
      ["notify", "here"],
    ]),
    "here",
  );
});

test("notifyModeForTags: unknown mode or missing tag reads as none", () => {
  assert.equal(notifyModeForTags([["notify", "everyone"]]), null);
  assert.equal(notifyModeForTags([["notify"]]), null);
  assert.equal(notifyModeForTags([["p", PUBKEY]]), null);
  assert.equal(notifyModeForTags([]), null);
});

test("notifyModeForTags: first notify tag wins", () => {
  assert.equal(
    notifyModeForTags([
      ["notify", "here"],
      ["notify", "channel"],
    ]),
    "here",
  );
});

test("@channel escalates regardless of presence or age", () => {
  const stale = makeEvent([["notify", "channel"]], {
    created_at: NOW - 86_400,
  });
  assert.equal(
    channelNotifyEscalates(stale, PUBKEY, {
      selfOnline: false,
      nowSeconds: NOW,
    }),
    true,
  );
});

test("@here escalates when online and fresh", () => {
  const event = makeEvent([["notify", "here"]]);
  assert.equal(channelNotifyEscalates(event, PUBKEY, LIVE), true);
});

test("@here does not escalate when offline", () => {
  const event = makeEvent([["notify", "here"]]);
  assert.equal(
    channelNotifyEscalates(event, PUBKEY, {
      selfOnline: false,
      nowSeconds: NOW,
    }),
    false,
  );
});

test("@here escalates at the freshness boundary and not beyond", () => {
  const atEdge = makeEvent([["notify", "here"]], {
    created_at: NOW - HERE_FRESHNESS_WINDOW_SECONDS,
  });
  const pastEdge = makeEvent([["notify", "here"]], {
    created_at: NOW - HERE_FRESHNESS_WINDOW_SECONDS - 1,
  });
  assert.equal(channelNotifyEscalates(atEdge, PUBKEY, LIVE), true);
  assert.equal(channelNotifyEscalates(pastEdge, PUBKEY, LIVE), false);
});

test("@here tolerates clock skew symmetrically", () => {
  const future = makeEvent([["notify", "here"]], {
    created_at: NOW + HERE_FRESHNESS_WINDOW_SECONDS,
  });
  const farFuture = makeEvent([["notify", "here"]], {
    created_at: NOW + HERE_FRESHNESS_WINDOW_SECONDS + 1,
  });
  assert.equal(channelNotifyEscalates(future, PUBKEY, LIVE), true);
  assert.equal(channelNotifyEscalates(farFuture, PUBKEY, LIVE), false);
});

test("author never escalates their own channel mention", () => {
  const own = makeEvent([["notify", "channel"]], { pubkey: PUBKEY });
  const ownUpper = makeEvent([["notify", "here"]], {
    pubkey: PUBKEY.toUpperCase(),
  });
  assert.equal(channelNotifyEscalates(own, PUBKEY, LIVE), false);
  assert.equal(channelNotifyEscalates(ownUpper, PUBKEY, LIVE), false);
});

test("no notify tag and empty pubkey never escalate", () => {
  assert.equal(channelNotifyEscalates(makeEvent(), PUBKEY, LIVE), false);
  assert.equal(
    channelNotifyEscalates(makeEvent([["notify", "channel"]]), "", LIVE),
    false,
  );
});
