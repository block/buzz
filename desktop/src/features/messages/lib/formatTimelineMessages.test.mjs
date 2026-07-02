import assert from "node:assert/strict";
import test from "node:test";

import {
  countTopLevelTimelineRows,
  formatTimelineMessages,
  isTimelineContentEvent,
} from "./formatTimelineMessages.ts";
import {
  CHANNEL_AUX_EVENT_KINDS,
  CHANNEL_TIMELINE_CONTENT_KINDS,
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";

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

function streamEdit(targetId, content, overrides = {}) {
  return {
    id: HEX64_B,
    pubkey: PUBKEY_A,
    kind: 40003,
    created_at: 1_700_000_001,
    content,
    tags: [
      ["h", CHANNEL_ID],
      ["e", targetId],
    ],
    sig: "sig",
    ...overrides,
  };
}

function huddleStarted(overrides = {}) {
  return {
    id: HEX64_B,
    pubkey: PUBKEY_A,
    kind: KIND_HUDDLE_STARTED,
    created_at: 1_700_000_001,
    content: JSON.stringify({
      ephemeral_channel_id: "8d764100-fd8f-44cf-9c98-6d8fbd739b8c",
    }),
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Keystone regression: aux events (edits/deletions) apply by `#e` reference,
// NOT by time-window overlap. This is the invariant the split-query +
// `#e`-backfill fix depends on: an edit/deletion can be loaded long after the
// message it targets — even with a far-future `created_at` — and must still
// apply. If the reducer ever gated aux application on timestamp proximity, a
// late edit/delete for a visible old message would silently render stale.
// ---------------------------------------------------------------------------

test("a far-future edit still rewrites the body of an old message", () => {
  const old = streamMessage({ created_at: 1_700_000_000 });
  const lateEdit = streamEdit(HEX64_A, "edited body", {
    created_at: 1_900_000_000,
  });
  const out = formatTimelineMessages([old, lateEdit], null, undefined, null);
  assert.equal(out.length, 1, "the message should still render");
  assert.equal(
    out[0].body,
    "edited body",
    "the far-future edit must overlay the old message's body regardless of the time gap",
  );
  assert.equal(out[0].edited, true, "the message must be marked edited");
});

test("a far-future deletion still hides an old message", () => {
  const old = streamMessage({ created_at: 1_700_000_000 });
  const lateDeletion = deletionEvent(9005, HEX64_A, {
    created_at: 1_900_000_000,
  });
  const out = formatTimelineMessages(
    [old, lateDeletion],
    null,
    undefined,
    null,
  );
  assert.equal(
    out.length,
    0,
    "the far-future deletion must filter out the old message regardless of the time gap",
  );
});

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

test("huddle start renders as a timeline row", () => {
  const out = formatTimelineMessages([huddleStarted()], null, undefined, null);
  assert.equal(out.length, 1);
  assert.equal(out[0].kind, KIND_HUDDLE_STARTED);
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
// Reaction pill ordering — pills must sort left→right by when each emoji was
// first added (ascending created_at), independent of input event order.
// ---------------------------------------------------------------------------

test("reaction pills sort by earliest created_at ascending", () => {
  const MSG_ID =
    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
  const message = {
    id: MSG_ID,
    pubkey: PUBKEY_A,
    kind: 9,
    created_at: 1_700_000_000,
    content: "hello",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
  };

  // 🎉 was added first (t=1001), 👍 was added second (t=1002).
  function reactionEvent(id, emoji, createdAt) {
    return {
      id,
      pubkey: PUBKEY_B,
      kind: 7,
      created_at: createdAt,
      content: emoji,
      tags: [
        ["h", CHANNEL_ID],
        ["e", MSG_ID],
      ],
      sig: "sig",
    };
  }

  const confetti = reactionEvent(`d${"d".repeat(63)}`, "🎉", 1_700_001_001);
  const thumbsUp = reactionEvent(`e${"e".repeat(63)}`, "👍", 1_700_001_002);

  // Feed ascending (🎉 first) — pills should be [🎉, 👍]
  const ascending = formatTimelineMessages(
    [message, confetti, thumbsUp],
    null,
    undefined,
    null,
  );
  assert.deepEqual(
    ascending[0].reactions?.map((r) => r.emoji),
    ["🎉", "👍"],
    "ascending input: 🎉 must come before 👍",
  );

  // Feed descending (👍 first in array) — order must be identical
  const descending = formatTimelineMessages(
    [message, thumbsUp, confetti],
    null,
    undefined,
    null,
  );
  assert.deepEqual(
    descending[0].reactions?.map((r) => r.emoji),
    ["🎉", "👍"],
    "descending input: pill order must be invariant to event array order",
  );
});

test("reaction pills with equal created_at tiebreak deterministically on emoji string", () => {
  const MSG_ID =
    "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
  const message = {
    id: MSG_ID,
    pubkey: PUBKEY_A,
    kind: 9,
    created_at: 1_700_000_000,
    content: "hello",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
  };
  const SAME_TS = 1_700_001_000;
  const reactionA = {
    id: `f${"f".repeat(63)}`,
    pubkey: PUBKEY_B,
    kind: 7,
    created_at: SAME_TS,
    content: "👍",
    tags: [
      ["h", CHANNEL_ID],
      ["e", MSG_ID],
    ],
    sig: "sig",
  };
  const reactionB = {
    id: `a${"a".repeat(63)}`,
    pubkey: PUBKEY_A,
    kind: 7,
    created_at: SAME_TS,
    content: "🎉",
    tags: [
      ["h", CHANNEL_ID],
      ["e", MSG_ID],
    ],
    sig: "sig",
  };

  const out1 = formatTimelineMessages(
    [message, reactionA, reactionB],
    null,
    undefined,
    null,
  );
  const out2 = formatTimelineMessages(
    [message, reactionB, reactionA],
    null,
    undefined,
    null,
  );

  assert.deepEqual(
    out1[0].reactions?.map((r) => r.emoji),
    out2[0].reactions?.map((r) => r.emoji),
    "equal timestamps: pill order must be identical regardless of input order",
  );
});

test("reaction pill order is invariant to duplicate same-actor same-emoji delivery order", () => {
  // Nostr can deliver the same reaction event twice (or relay redelivery can
  // produce two events with the same target/actor/emoji but different ids/timestamps).
  // The pill sort key must be the EARLIEST createdAt seen, not the last-written.
  const MSG_ID =
    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
  const message = {
    id: MSG_ID,
    pubkey: PUBKEY_A,
    kind: 9,
    created_at: 1_700_000_000,
    content: "hello",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
  };
  // 🎉 first at t=1001 (the canonical earliest), then a duplicate at t=1005.
  // 👍 arrives at t=1003 — must still be to the right of 🎉.
  const confettiFirst = {
    id: `c${"c".repeat(63)}`,
    pubkey: PUBKEY_B,
    kind: 7,
    created_at: 1_700_001_001,
    content: "🎉",
    tags: [
      ["h", CHANNEL_ID],
      ["e", MSG_ID],
    ],
    sig: "sig",
  };
  const confettiDupe = {
    id: `d${"d".repeat(63)}`,
    pubkey: PUBKEY_B,
    kind: 7,
    created_at: 1_700_001_005,
    content: "🎉",
    tags: [
      ["h", CHANNEL_ID],
      ["e", MSG_ID],
    ],
    sig: "sig",
  };
  const thumbsUp = {
    id: `e${"e".repeat(63)}`,
    pubkey: PUBKEY_B,
    kind: 7,
    created_at: 1_700_001_003,
    content: "👍",
    tags: [
      ["h", CHANNEL_ID],
      ["e", MSG_ID],
    ],
    sig: "sig",
  };

  // Dupe arrives BEFORE canonical — naive last-write-wins would store t=1001
  // (from confettiFirst which comes second), giving correct order by accident.
  const dupeFirst = formatTimelineMessages(
    [message, confettiDupe, confettiFirst, thumbsUp],
    null,
    undefined,
    null,
  );
  // Dupe arrives AFTER canonical — last-write-wins stores t=1005 (the dupe),
  // which would make 🎉's sort key t=1005 > 👍's t=1003, incorrectly reversing order.
  const dupeLast = formatTimelineMessages(
    [message, confettiFirst, thumbsUp, confettiDupe],
    null,
    undefined,
    null,
  );

  assert.deepEqual(
    dupeFirst[0].reactions?.map((r) => r.emoji),
    ["🎉", "👍"],
    "🎉 (earliest at t=1001) must stay left of 👍 (t=1003) when dupe arrives first",
  );
  assert.deepEqual(
    dupeLast[0].reactions?.map((r) => r.emoji),
    ["🎉", "👍"],
    "🎉 (earliest at t=1001) must stay left of 👍 (t=1003) when dupe arrives last",
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

test("countTopLevelTimelineRows counts huddle start rows", () => {
  assert.equal(countTopLevelTimelineRows([huddleStarted()]), 1);
});

test("huddle ended stays lifecycle-only, not a timeline row", () => {
  assert.equal(isTimelineContentEvent({ kind: KIND_HUDDLE_ENDED }), false);
});

// Guardrail: the history fetch requests exactly CHANNEL_TIMELINE_CONTENT_KINDS,
// so that set must stay in lockstep with isTimelineContentEvent. Drift would
// silently drop a content kind from history (fetched but never rendered) or
// fetch an aux kind as content. Assert parity in both directions.
test("CHANNEL_TIMELINE_CONTENT_KINDS matches isTimelineContentEvent", () => {
  for (const kind of CHANNEL_TIMELINE_CONTENT_KINDS) {
    assert.ok(
      isTimelineContentEvent({ kind }),
      `content kind ${kind} must be a timeline content event`,
    );
  }
  for (const kind of CHANNEL_AUX_EVENT_KINDS) {
    assert.ok(
      !isTimelineContentEvent({ kind }),
      `aux kind ${kind} must not be a timeline content event`,
    );
  }
});

// --- agentOwner: duplicate-agent-name disambiguation ---------------------

const OWNER_PUBKEY =
  "3333333333333333333333333333333333333333333333333333333333333333";
const AGENT_PUBKEY_A =
  "4444444444444444444444444444444444444444444444444444444444444444";
const AGENT_PUBKEY_B =
  "5555555555555555555555555555555555555555555555555555555555555555";

function botMember(pubkey, displayName) {
  return {
    pubkey,
    role: "bot",
    isAgent: true,
    joinedAt: "2026-01-01T00:00:00Z",
    displayName,
  };
}

function agentProfiles() {
  return {
    [AGENT_PUBKEY_A]: {
      displayName: "Bart",
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: OWNER_PUBKEY,
      isAgent: true,
    },
    [AGENT_PUBKEY_B]: {
      displayName: "Bart",
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: PUBKEY_B,
      isAgent: true,
    },
    [OWNER_PUBKEY]: {
      displayName: "Taylor",
      avatarUrl: "https://example.com/taylor.png",
      nip05Handle: null,
      ownerPubkey: null,
    },
  };
}

test("agentOwner set when two agents share a display name", () => {
  const [message] = formatTimelineMessages(
    [streamMessage({ pubkey: AGENT_PUBKEY_A })],
    null,
    undefined,
    null,
    agentProfiles(),
    [botMember(AGENT_PUBKEY_A, "Bart"), botMember(AGENT_PUBKEY_B, "Bart")],
  );
  assert.deepEqual(message.agentOwner, {
    pubkey: OWNER_PUBKEY,
    displayName: "Taylor",
    avatarUrl: "https://example.com/taylor.png",
  });
});

test("agentOwner omitted for a uniquely named agent", () => {
  const profiles = agentProfiles();
  profiles[AGENT_PUBKEY_B] = {
    ...profiles[AGENT_PUBKEY_B],
    displayName: "Lisa",
  };
  const [message] = formatTimelineMessages(
    [streamMessage({ pubkey: AGENT_PUBKEY_A })],
    null,
    undefined,
    null,
    profiles,
    [botMember(AGENT_PUBKEY_A, "Bart"), botMember(AGENT_PUBKEY_B, "Lisa")],
  );
  assert.equal(message.agentOwner, undefined);
});

test("agentOwner omitted for human authors even on name collision", () => {
  const profiles = agentProfiles();
  profiles[PUBKEY_A] = {
    displayName: "Bart",
    avatarUrl: null,
    nip05Handle: null,
    ownerPubkey: null,
  };
  const [message] = formatTimelineMessages(
    [streamMessage({ pubkey: PUBKEY_A })],
    null,
    undefined,
    null,
    profiles,
    [
      { ...botMember(PUBKEY_A, "Bart"), role: "member", isAgent: false },
      botMember(AGENT_PUBKEY_A, "Bart"),
      botMember(AGENT_PUBKEY_B, "Bart"),
    ],
  );
  assert.equal(message.agentOwner, undefined);
});

test("agentOwner omitted when the agent has no ownerPubkey", () => {
  const profiles = agentProfiles();
  profiles[AGENT_PUBKEY_A] = {
    ...profiles[AGENT_PUBKEY_A],
    ownerPubkey: null,
  };
  const [message] = formatTimelineMessages(
    [streamMessage({ pubkey: AGENT_PUBKEY_A })],
    null,
    undefined,
    null,
    profiles,
    [botMember(AGENT_PUBKEY_A, "Bart"), botMember(AGENT_PUBKEY_B, "Bart")],
  );
  assert.equal(message.agentOwner, undefined);
});

test("agentOwner falls back to null profile fields for unknown owner", () => {
  const profiles = agentProfiles();
  delete profiles[OWNER_PUBKEY];
  const [message] = formatTimelineMessages(
    [streamMessage({ pubkey: AGENT_PUBKEY_A })],
    null,
    undefined,
    null,
    profiles,
    [botMember(AGENT_PUBKEY_A, "Bart"), botMember(AGENT_PUBKEY_B, "Bart")],
  );
  assert.deepEqual(message.agentOwner, {
    pubkey: OWNER_PUBKEY,
    displayName: null,
    avatarUrl: null,
  });
});

test("agent name matching is case-insensitive", () => {
  const profiles = agentProfiles();
  profiles[AGENT_PUBKEY_B] = {
    ...profiles[AGENT_PUBKEY_B],
    displayName: "bart",
  };
  const [message] = formatTimelineMessages(
    [streamMessage({ pubkey: AGENT_PUBKEY_A })],
    null,
    undefined,
    null,
    profiles,
    [botMember(AGENT_PUBKEY_A, "Bart"), botMember(AGENT_PUBKEY_B, "bart")],
  );
  assert.equal(message.agentOwner?.pubkey, OWNER_PUBKEY);
});
