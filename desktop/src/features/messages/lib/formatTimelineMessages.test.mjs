import assert from "node:assert/strict";
import test from "node:test";

import {
  countTopLevelTimelineRows,
  formatTimelineMessages,
} from "./formatTimelineMessages.ts";

const HEX64_A =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEX64_B =
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PUBKEY_A =
  "1111111111111111111111111111111111111111111111111111111111111111";
const PUBKEY_B =
  "2222222222222222222222222222222222222222222222222222222222222222";
const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";

function streamMessage(overrides = {}) {
  return {
    id: HEX64_A,
    pubkey: PUBKEY_A,
    kind: 9,
    created_at: 1_700_000_000,
    content: "hello world",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
    ...overrides,
  };
}

function deletionEvent(kind, targetId, overrides = {}) {
  return {
    id: HEX64_B,
    pubkey: PUBKEY_B,
    kind,
    created_at: 1_700_000_001,
    content: "",
    tags: [
      ["h", CHANNEL_ID],
      ["e", targetId],
    ],
    sig: "sig",
    ...overrides,
  };
}

test("kind:5 (NIP-09) deletion hides the target message", () => {
  const events = [streamMessage(), deletionEvent(5, HEX64_A)];
  const out = formatTimelineMessages(events, null, undefined, null);
  assert.equal(
    out.length,
    0,
    "the kind:9 message should be filtered out by the kind:5 deletion",
  );
});

test("kind:9005 (NIP-29 / Buzz-native) deletion hides the target message", () => {
  // This is the actual reported bug: agents emit kind:9005 deletes via the
  // CLI. Without recognizing 9005 as a deletion marker the message stayed
  // rendered until manual refresh.
  const events = [streamMessage(), deletionEvent(9005, HEX64_A)];
  const out = formatTimelineMessages(events, null, undefined, null);
  assert.equal(
    out.length,
    0,
    "the kind:9 message should be filtered out by the kind:9005 deletion",
  );
});

test("non-deletion event kinds do NOT hide the target message", () => {
  // Sanity check: only kind:5 and kind:9005 are treated as deletion markers.
  // A kind:7 reaction with the same `e` tag must not erase the target.
  const reaction = {
    id: HEX64_B,
    pubkey: PUBKEY_B,
    kind: 7,
    created_at: 1_700_000_001,
    content: "+",
    tags: [
      ["h", CHANNEL_ID],
      ["e", HEX64_A],
    ],
    sig: "sig",
  };
  const events = [streamMessage(), reaction];
  const out = formatTimelineMessages(events, null, undefined, null);
  assert.equal(out.length, 1, "the kind:9 message should still be visible");
});

test("deletion target with non-hex `e` tag value is ignored", () => {
  const bogusDeletion = deletionEvent(9005, HEX64_A, {
    tags: [
      ["h", CHANNEL_ID],
      ["e", "not-hex"],
    ],
  });
  const events = [streamMessage(), bogusDeletion];
  const out = formatTimelineMessages(events, null, undefined, null);
  assert.equal(
    out.length,
    1,
    "malformed deletion tag should not match anything",
  );
});

// ---------------------------------------------------------------------------
// countTopLevelTimelineRows — the unit fetch-older pages by. Must match the
// rows `buildMainTimelineEntries` would actually render: top-level content
// events, minus deletions, with thread replies collapsed into their parent.
// ---------------------------------------------------------------------------

function hex64(char) {
  return char.repeat(64);
}

function message(id, overrides = {}) {
  return {
    id,
    pubkey: PUBKEY_A,
    kind: 9,
    created_at: 1_700_000_000,
    content: "hi",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
    ...overrides,
  };
}

function reply(id, parentId, overrides = {}) {
  return message(id, {
    tags: [
      ["h", CHANNEL_ID],
      ["e", parentId, "", "reply"],
    ],
    ...overrides,
  });
}

test("countTopLevelTimelineRows counts top-level messages", () => {
  const events = [
    message(hex64("1")),
    message(hex64("2")),
    message(hex64("3")),
  ];
  assert.equal(countTopLevelTimelineRows(events), 3);
});

test("countTopLevelTimelineRows ignores collapsed thread replies", () => {
  const root = hex64("1");
  const events = [
    message(root),
    reply(hex64("2"), root),
    reply(hex64("3"), root),
  ];
  // Two replies collapse into the root's summary → one visible row.
  assert.equal(countTopLevelTimelineRows(events), 1);
});

test("countTopLevelTimelineRows counts broadcast replies as top-level", () => {
  const root = hex64("1");
  const broadcast = reply(hex64("2"), root, {
    tags: [
      ["h", CHANNEL_ID],
      ["e", root, "", "reply"],
      ["broadcast", "1"],
    ],
  });
  assert.equal(countTopLevelTimelineRows([message(root), broadcast]), 2);
});

test("countTopLevelTimelineRows excludes deleted messages", () => {
  const target = hex64("1");
  const events = [
    message(target),
    message(hex64("2")),
    deletionEvent(9005, target, { id: hex64("9") }),
  ];
  assert.equal(countTopLevelTimelineRows(events), 1);
});

test("countTopLevelTimelineRows ignores non-content kinds (reactions)", () => {
  const reaction = {
    id: hex64("9"),
    pubkey: PUBKEY_B,
    kind: 7,
    created_at: 1_700_000_001,
    content: "+",
    tags: [
      ["h", CHANNEL_ID],
      ["e", hex64("1")],
    ],
    sig: "sig",
  };
  assert.equal(countTopLevelTimelineRows([message(hex64("1")), reaction]), 1);
});
