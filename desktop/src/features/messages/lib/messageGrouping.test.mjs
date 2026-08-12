import assert from "node:assert/strict";
import test from "node:test";

import {
  MESSAGE_GROUPING_WINDOW_SECONDS,
  hasSameMessageAuthor,
  isWithinGroupingWindow,
  startsNewMessageGroup,
} from "./messageGrouping.ts";

test("startsNewMessageGroup: sent-from-thread messages start a fresh group", () => {
  assert.equal(
    startsNewMessageGroup({
      tags: [["buzz:sent-from-thread", "root-event", "Root summary"]],
    }),
    true,
  );
  assert.equal(startsNewMessageGroup({ tags: [["h", "channel-id"]] }), false);
  assert.equal(startsNewMessageGroup(undefined), false);
});

test("hasSameMessageAuthor: matches case-insensitively and trims", () => {
  assert.equal(
    hasSameMessageAuthor({ pubkey: " ABC " }, { pubkey: "abc" }),
    true,
  );
  assert.equal(
    hasSameMessageAuthor({ pubkey: "abc" }, { pubkey: "def" }),
    false,
  );
});

test("hasSameMessageAuthor: missing pubkeys never match", () => {
  assert.equal(hasSameMessageAuthor(null, { pubkey: "abc" }), false);
  assert.equal(hasSameMessageAuthor({ pubkey: "abc" }, undefined), false);
  assert.equal(hasSameMessageAuthor({ pubkey: "" }, { pubkey: "" }), false);
});

test("hasSameMessageAuthor: raw signer changes break displayed-author grouping", () => {
  assert.equal(
    hasSameMessageAuthor(
      { pubkey: "actor", signerPubkey: "relay" },
      { pubkey: "actor", signerPubkey: "actor" },
    ),
    false,
  );
  assert.equal(
    hasSameMessageAuthor(
      { pubkey: " ACTOR ", signerPubkey: " RELAY " },
      { pubkey: "actor", signerPubkey: "relay" },
    ),
    true,
  );
});

test("hasSameMessageAuthor: missing signer falls back to displayed author", () => {
  assert.equal(
    hasSameMessageAuthor(
      { pubkey: "actor" },
      { pubkey: "actor", signerPubkey: "actor" },
    ),
    true,
  );
});

test("isWithinGroupingWindow: at or under the boundary is in window", () => {
  const base = 1_000_000;
  assert.equal(isWithinGroupingWindow(base, base), true);
  assert.equal(
    isWithinGroupingWindow(base, base + MESSAGE_GROUPING_WINDOW_SECONDS),
    true,
  );
});

test("isWithinGroupingWindow: past the boundary is out of window", () => {
  const base = 1_000_000;
  assert.equal(
    isWithinGroupingWindow(base, base + MESSAGE_GROUPING_WINDOW_SECONDS + 1),
    false,
  );
});

test("isWithinGroupingWindow: out-of-order (negative gap) is out of window", () => {
  const base = 1_000_000;
  assert.equal(isWithinGroupingWindow(base + 60, base), false);
});

test("isWithinGroupingWindow: missing timestamps are out of window", () => {
  assert.equal(isWithinGroupingWindow(null, 1_000_000), false);
  assert.equal(isWithinGroupingWindow(1_000_000, undefined), false);
  assert.equal(isWithinGroupingWindow(undefined, undefined), false);
});
