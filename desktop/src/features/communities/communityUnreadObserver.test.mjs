import assert from "node:assert/strict";
import test from "node:test";

import {
  extractHiddenDmIds,
  extractMemberChannelIds,
  fetchCommunityUnread,
  resolveObservedChannels,
} from "./communityUnreadObserver.ts";

const PUBKEY = "a".repeat(64);
const OTHER = "b".repeat(64);
const CHANNEL_ID = "channel-1";
const THREAD_ROOT = "c".repeat(64);
const THREAD_ROOT_2 = "d".repeat(64);

const EMPTY_RELATIONSHIPS = {
  participatedRootIds: new Set(),
  followedRootIds: new Set(),
  authoredRootIds: new Set(),
  mutedRootIds: new Set(),
};

function readRelationships(overrides = {}) {
  return () => ({ ...EMPTY_RELATIONSHIPS, ...overrides });
}

function event(overrides = {}) {
  return {
    id: overrides.id ?? `${Math.random()}`.padEnd(64, "0").slice(0, 64),
    pubkey: overrides.pubkey ?? OTHER,
    created_at: overrides.created_at ?? 100,
    kind: overrides.kind ?? 9,
    tags: overrides.tags ?? [],
    content: overrides.content ?? "",
    sig: overrides.sig ?? "sig",
  };
}

function relayFor(filters) {
  return {
    requests: [],
    async fetchEvents(filter) {
      this.requests.push(filter);
      return filters.shift()?.(filter) ?? [];
    },
  };
}

// Helper: encode a mutes payload as JSON (decryptMutes stub returns content as-is)
function mutesContent(mutedIds) {
  const channels = {};
  for (const id of mutedIds) {
    channels[id] = { muted: true, updatedAt: 1 };
  }
  return JSON.stringify({ version: 1, channels });
}

test("extractMemberChannelIds deduplicates d tags", () => {
  assert.deepEqual(
    extractMemberChannelIds([
      event({
        tags: [
          ["d", "one"],
          ["d", "two"],
        ],
      }),
      event({ tags: [["d", "one"]] }),
    ]),
    ["one", "two"],
  );
});

test("resolveObservedChannels uses latest metadata and archived flag", () => {
  assert.deepEqual(
    resolveObservedChannels(
      ["stream", "dm", "missing"],
      [
        event({
          created_at: 1,
          tags: [
            ["d", "dm"],
            ["t", "stream"],
          ],
        }),
        event({
          created_at: 2,
          tags: [
            ["d", "dm"],
            ["t", "dm"],
          ],
        }),
        event({
          tags: [
            ["d", "stream"],
            ["archived", "true"],
          ],
        }),
      ],
    ),
    [
      { id: "stream", channelType: "stream", archived: true },
      { id: "dm", channelType: "dm", archived: false },
      { id: "missing", channelType: "stream", archived: false },
    ],
  );
});

test("extractHiddenDmIds reads h tags from latest visibility snapshot", () => {
  assert.deepEqual(
    extractHiddenDmIds([
      event({ created_at: 1, tags: [["h", "old"]] }),
      event({
        created_at: 2,
        tags: [
          ["h", "new"],
          ["h", "other"],
        ],
      }),
    ]),
    new Set(["new", "other"]),
  );
});

test("fetchCommunityUnread returns dot and mention count without total unread count", async () => {
  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events (parallel with metadata)
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [],
    // 5. mutes events (parallel with read-state)
    () => [],
    // 6. unread events
    () => [
      event({
        id: "unread".padEnd(64, "0"),
        created_at: 20,
        tags: [["h", CHANNEL_ID]],
      }),
    ],
    // 7. mention events
    () => [
      event({
        id: "mention".padEnd(64, "0"),
        created_at: 30,
        tags: [
          ["h", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async (value) => value,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 1 });
  assert.equal(relay.requests.at(-1)["#p"][0], PUBKEY);
});

test("fetchCommunityUnread ignores self-authored and read thread/message events", async () => {
  const threadReply = event({
    id: "reply".padEnd(64, "0"),
    created_at: 50,
    tags: [
      ["h", CHANNEL_ID],
      ["e", THREAD_ROOT, "", "root"],
      ["e", "parent".padEnd(64, "0"), "", "reply"],
      ["p", PUBKEY],
    ],
  });
  const selfMention = event({
    id: "self".padEnd(64, "0"),
    pubkey: PUBKEY,
    created_at: 70,
    tags: [
      ["h", CHANNEL_ID],
      ["p", PUBKEY],
    ],
  });

  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events (parallel with metadata)
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [
      event({
        pubkey: PUBKEY,
        created_at: 80,
        tags: [
          ["d", "read-state:a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"],
          ["t", "read-state"],
        ],
        content: JSON.stringify({
          v: 1,
          client_id: "client",
          contexts: {
            [CHANNEL_ID]: 10,
            [`thread:${THREAD_ROOT}`]: 60,
          },
        }),
      }),
    ],
    // 5. mutes events (parallel with read-state)
    () => [],
    // 6. unread events
    () => [threadReply, selfMention],
    // 7. mention events
    () => [threadReply, selfMention],
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async (value) => value,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchCommunityUnread excludes muted-only channel — returns hasUnread:false mentionCount:0", async () => {
  const MUTED_CHANNEL = "muted-channel-1";

  const relay = relayFor([
    // 1. member events — one muted channel
    () => [
      event({
        tags: [
          ["d", MUTED_CHANNEL],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", MUTED_CHANNEL],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events (parallel with metadata)
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [],
    // 5. mutes events — MUTED_CHANNEL is muted
    () => [
      event({
        pubkey: PUBKEY,
        content: mutesContent([MUTED_CHANNEL]),
      }),
    ],
    // No per-channel fetches should follow — muted channel is skipped
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async (value) => value,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchCommunityUnread counts unmuted channel but skips muted channel", async () => {
  const UNMUTED_CHANNEL = "channel-unmuted";
  const MUTED_CHANNEL = "channel-muted";

  const relay = relayFor([
    // 1. member events — two channels
    () => [
      event({
        tags: [
          ["d", UNMUTED_CHANNEL],
          ["d", MUTED_CHANNEL],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", UNMUTED_CHANNEL],
          ["t", "stream"],
        ],
      }),
      event({
        tags: [
          ["d", MUTED_CHANNEL],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events (parallel with metadata)
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [],
    // 5. mutes events — only MUTED_CHANNEL is muted
    () => [
      event({
        pubkey: PUBKEY,
        content: mutesContent([MUTED_CHANNEL]),
      }),
    ],
    // 6. unread events for UNMUTED_CHANNEL (muted channel loop iteration never fires)
    () => [
      event({
        id: "unread".padEnd(64, "0"),
        created_at: 20,
        tags: [["h", UNMUTED_CHANNEL]],
      }),
    ],
    // 7. mention events for UNMUTED_CHANNEL
    () => [
      event({
        id: "mention".padEnd(64, "0"),
        created_at: 30,
        tags: [
          ["h", UNMUTED_CHANNEL],
          ["p", PUBKEY],
        ],
      }),
    ],
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async (value) => value,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 1 });
});

test("fetchCommunityUnread treats decryption failure as empty mutes set", async () => {
  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events (parallel with metadata)
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [],
    // 5. mutes events — present but decryption will throw
    () => [
      event({
        pubkey: PUBKEY,
        content: "corrupted-ciphertext",
      }),
    ],
    // 6. unread events — channel is NOT muted (decryption failed → empty set)
    () => [
      event({
        id: "unread".padEnd(64, "0"),
        created_at: 20,
        tags: [["h", CHANNEL_ID]],
      }),
    ],
    // 7. mention events
    () => [],
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async () => {
      throw new Error("decryption failed");
    },
    readThreadRelationships: readRelationships(),
  });

  // Channel counted as if no mutes
  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchCommunityUnread treats absent mutes blob as empty mutes set", async () => {
  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events (parallel with metadata)
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [],
    // 5. mutes events — none
    () => [],
    // 6. unread events
    () => [
      event({
        id: "unread".padEnd(64, "0"),
        created_at: 20,
        tags: [["h", CHANNEL_ID]],
      }),
    ],
    // 7. mention events
    () => [],
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async (value) => value,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

// ── Thread-relevance gate tests ────────────────────────────────────────────

function threadedReplyEvent(overrides = {}) {
  return event({
    id: overrides.id ?? "reply".padEnd(64, "0"),
    created_at: overrides.created_at ?? 20,
    pubkey: overrides.pubkey ?? OTHER,
    tags: [
      ["h", CHANNEL_ID],
      ["e", THREAD_ROOT_2, "", "root"],
      ["e", "parent".padEnd(64, "0"), "", "reply"],
      ...(overrides.extraTags ?? []),
    ],
    ...overrides,
  });
}

function baseRelay(unreadEvent, mutesPayload = null) {
  return relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events (parallel with metadata)
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [],
    // 5. mutes events
    () =>
      mutesPayload ? [event({ pubkey: PUBKEY, content: mutesPayload })] : [],
    // 6. unread events — the single event under test
    () => [unreadEvent],
    // 7. mention events
    () => [],
  ]);
}

const THREAD_GATE_CASES = [
  {
    name: "threaded reply in untracked root → hasUnread:false",
    unreadEvent: threadedReplyEvent(),
    relationships: {},
    expected: { hasUnread: false, mentionCount: 0 },
  },
  {
    name: "threaded reply in participatedRootIds → hasUnread:true",
    unreadEvent: threadedReplyEvent(),
    relationships: { participatedRootIds: new Set([THREAD_ROOT_2]) },
    expected: { hasUnread: true, mentionCount: 0 },
  },
  {
    name: "#p-mention reply in untracked root → hasUnread:true (mention overrides)",
    unreadEvent: threadedReplyEvent({
      id: "mention-reply".padEnd(64, "0"),
      extraTags: [["p", PUBKEY]],
    }),
    relationships: {},
    expected: { hasUnread: true, mentionCount: 0 },
  },
  {
    name: "top-level post → hasUnread:true (no thread gate)",
    unreadEvent: event({
      id: "toplevel".padEnd(64, "0"),
      created_at: 20,
      tags: [["h", CHANNEL_ID]],
    }),
    relationships: {},
    expected: { hasUnread: true, mentionCount: 0 },
  },
  {
    name: "threaded reply whose root is in mutedRootIds → hasUnread:false",
    unreadEvent: threadedReplyEvent({ id: "muted-reply".padEnd(64, "0") }),
    relationships: {
      participatedRootIds: new Set([THREAD_ROOT_2]),
      mutedRootIds: new Set([THREAD_ROOT_2]),
    },
    expected: { hasUnread: false, mentionCount: 0 },
  },
];

for (const {
  name,
  unreadEvent,
  relationships,
  expected,
} of THREAD_GATE_CASES) {
  test(`fetchCommunityUnread ${name}`, async () => {
    const relay = baseRelay(unreadEvent);
    const result = await fetchCommunityUnread({
      client: relay,
      pubkey: PUBKEY,
      nowSeconds: 100,
      decryptReadState: async (v) => v,
      decryptMutes: async (v) => v,
      readThreadRelationships: readRelationships(relationships),
    });
    assert.deepEqual(result, expected);
  });
}

// ── Override register authoritative source tests ──────────────────────────

// A relay that serves one channel (CHANNEL_ID) as a member channel,
// with no read state and no incoming events.
function quietRelay() {
  return relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [],
    // 5. mutes events
    () => [],
    // 6. unread events — none
    () => [],
    // 7. mention events — none
    () => [],
  ]);
}

// A relay that serves one channel (CHANNEL_ID) with a synced read marker at
// the given unix-second timestamp and no new incoming events.
function quietRelayWithReadState(readAtSeconds) {
  return relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events (parallel with visibility)
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events
    () => [],
    // 4. read-state events (parallel with mutes)
    () => [
      event({
        pubkey: PUBKEY,
        created_at: 200,
        tags: [
          ["d", "read-state:a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"],
          ["t", "read-state"],
        ],
        content: JSON.stringify({
          v: 1,
          client_id: "client",
          contexts: { [CHANNEL_ID]: readAtSeconds },
        }),
      }),
    ],
    // 5. mutes events
    () => [],
    // 6. unread events — none (marker covers everything)
    () => [],
    // 7. mention events — none
    () => [],
  ]);
}

test("fetchCommunityUnread stored active register lights rail (hasUnread:true)", async () => {
  // Authoritative source: complete projection with active register (S=1,C=0,B=0 with F=0).
  // The fetch returns no override events — projection is the authority.
  const relay = quietRelay();
  // Active register: S=1, C=0, B=0 (baseline), frontier=0 → override_active = true
  const overrides = new Map([[CHANNEL_ID, { s: 1, c: 0, b: 0 }]]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    getProjection: () => ({
      loadComplete: true,
      frontiers: new Map(),
      overrides,
    }),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchCommunityUnread stored tombstoned register does NOT light rail (hasUnread:false)", async () => {
  // Authoritative source: complete projection with tombstoned register (S=1,C=1 → inactive).
  // Must not light the dot.
  const relay = quietRelay();
  // Tombstoned: S=1, C=1, B=0 → override_active = false (S > C fails: 1 > 1 = false)
  const overrides = new Map([[CHANNEL_ID, { s: 1, c: 1, b: 0 }]]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    getProjection: () => ({
      loadComplete: true,
      frontiers: new Map(),
      overrides,
    }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchCommunityUnread old active register beyond horizon still lights rail", async () => {
  // An active register that is older than the 7-day horizon must still light
  // the rail. The fetch is tag-free and has no `since`, so it returns no
  // events here, but the complete projection is the authority.
  const relay = quietRelay();
  const VERY_OLD_FRONTIER = 1; // far beyond any 7-day horizon
  // S=1, C=0, B=VERY_OLD_FRONTIER, and frontier=VERY_OLD_FRONTIER → active
  const overrides = new Map([
    [CHANNEL_ID, { s: 1, c: 0, b: VERY_OLD_FRONTIER }],
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100 + 7 * 24 * 3600 + 1, // past the old 7-day cutoff
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    getProjection: () => ({
      loadComplete: true,
      frontiers: new Map(),
      overrides,
    }),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchCommunityUnread no stored register and empty fetch → falls through to relay gate", async () => {
  // No stored registers, no override event fetched, but a real unread event exists.
  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility
    () => [],
    // 4. read-state — empty
    () => [],
    // 5. mutes
    () => [],
    // 6. unread events — one real unread
    () => [
      event({
        id: "real-unread".padEnd(64, "0"),
        created_at: 20,
        tags: [["h", CHANNEL_ID]],
      }),
    ],
    // 7. mention events
    () => [],
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // No projection — no override registers; falls through to relay evidence gate
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchCommunityUnread channel not in member list → hasUnread:false", async () => {
  // No members — register lookup is never reached.
  const relay = relayFor([
    () => [], // member events empty
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    getProjection: () => ({
      loadComplete: true,
      frontiers: new Map(),
      overrides: new Map([[CHANNEL_ID, { s: 1, c: 0, b: 0 }]]),
    }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchCommunityUnread active register muted channel → hasUnread:false", async () => {
  // Override is active, but the channel is muted — mute wins.
  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility
    () => [],
    // 4. read-state
    () => [],
    // 5. mutes — CHANNEL_ID is muted
    () => [
      event({
        pubkey: PUBKEY,
        content: mutesContent([CHANNEL_ID]),
      }),
    ],
    // No per-channel fetches expected — muted channel is skipped
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // Active register, but channel is muted — mute wins
    getProjection: () => ({
      loadComplete: true,
      frontiers: new Map(),
      overrides: new Map([[CHANNEL_ID, { s: 1, c: 0, b: 0 }]]),
    }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchCommunityUnread frontier advance deactivates stored register → hasUnread:false", async () => {
  // Register: S=1, C=0, B=50 (active only when F<=50).
  // Fetched frontier: 100 > 50 → override_active = false.
  const relay = quietRelayWithReadState(100);
  // B=50 means active only when F<=50; fetched frontier is 100 → inactive
  const overrides = new Map([[CHANNEL_ID, { s: 1, c: 0, b: 50 }]]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 200,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    getProjection: () => ({
      loadComplete: true,
      frontiers: new Map(),
      overrides,
    }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

// ── Adversarial witness tests ─────────────────────────────────────────────────
//
// These witnesses verify that the rail's override evaluation cannot resurrect
// a superseded register or produce a false verdict from a torn/partial state.

test("fetchCommunityUnread adversarial-1: stale live register + fetched tombstone → dark when no complete projection", async () => {
  // The OLD per-field max join would: max(stale_live, tombstone) = live (resurrection).
  // Without a complete projection, the fetched coordinate-deduped tombstone is
  // authoritative; the register is inactive and the rail must remain dark.
  //
  // Setup: fetched event carries tombstone (S=0,C=4,B=0) for CHANNEL_ID.
  const SLOT = "a".repeat(32); // valid 32-hex slot
  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility
    () => [],
    // 4. read-state: tombstone event (S=0, C=4, B=0 → inactive, S not > C)
    () => [
      event({
        pubkey: PUBKEY,
        created_at: 300,
        tags: [
          ["d", `read-state:${SLOT}`],
          ["t", "read-state"],
        ],
        // Contexts blob: channel has tombstone register (ov_s=0 absent means S=0, ov_c=4)
        content: JSON.stringify({
          v: 1,
          client_id: "client",
          contexts: {
            [CHANNEL_ID]: 50,
            [`ov_c:${CHANNEL_ID}`]: 4,
          },
        }),
      }),
    ],
    // 5. mutes
    () => [],
    // 6. unread events — none (the override is inactive, no message events either)
    () => [],
    // 7. mention events
    () => [],
  ]);

  // No complete projection — use fetched-only (no stored merge).
  // Previously this would be called with readStoredRegisters returning a stale
  // live register (S=5,C=0,B=100), which when max-joined with fetched tombstone
  // (S=0,C=4,B=0) produced (S=5,C=4,B=100) → active (resurrection).
  // New behavior: fetched state alone → (S=0,C=4,B=0) → inactive → dark.
  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 400,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // No getProjection: fetched-deduped is the sole authority.
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchCommunityUnread adversarial-2: projection with frontier deactivates register → dark", async () => {
  // CRITICAL 2 fix: the rail now reads projection.frontiers, not only fetched frontiers.
  // A register (S=5,C=0,B=100) with projection frontier F=101 must evaluate as
  // inactive (101 > 100) — previously the frontier was not in the stored projection,
  // so it defaulted to F=0 and the register appeared active.
  //
  // The fetched relay state returns empty read-state (frontier not in fetched events).
  // The projection has the committed frontier F=101 from a successful mark-read.
  const relay = quietRelay(); // no read-state events fetched

  const overrides = new Map([[CHANNEL_ID, { s: 5, c: 0, b: 100 }]]);
  // projection.frontiers has F=101 for CHANNEL_ID → deactivates the register
  const frontiers = new Map([[CHANNEL_ID, 101]]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 200,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    getProjection: () => ({ loadComplete: true, frontiers, overrides }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchCommunityUnread adversarial-5: complete_projection_live_register_not_darkened_by_stale_fetched_tombstone", async () => {
  // A fresh local mark-unread committed into the projection (S=5,C=0,B=100,F=50)
  // must NOT be darkened by an independently fetched tombstone (S=0,C=4,B=0).
  //
  // The observer cannot adjudicate cross-source ordering: the manager owns
  // coordinate dedupe and CRDT ingest.  A categorical tombstone-wins rule would
  // hide the user's mark-unread until the relay publish catches up (the
  // false-negative direction), which is the more harmful failure.
  //
  // When the projection is complete, projection.overrides is the SOLE liveness
  // authority.  The fetched register shape is irrelevant.  Any transient
  // false-positive resolves when the manager ingests the tombstone via its own
  // CRDT path.
  const SLOT = "a".repeat(32); // valid 32-hex slot
  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events
    () => [],
    // 4. read-state: tombstone wire format (ov_c present, ov_s absent → s=0,c=4,b=0)
    //    Frontier F=50 is also present so the projection frontier is not strictly
    //    newer — only the live-register vs concurrent-tombstone ordering is under test.
    () => [
      event({
        pubkey: PUBKEY,
        created_at: 400,
        tags: [
          ["d", `read-state:${SLOT}`],
          ["t", "read-state"],
        ],
        content: JSON.stringify({
          v: 1,
          client_id: "client",
          contexts: {
            [CHANNEL_ID]: 50,
            [`ov_c:${CHANNEL_ID}`]: 4,
            // ov_s: intentionally absent → tombstone (s=0)
          },
        }),
      }),
    ],
    // 5. mutes
    () => [],
    // 6. unread events — none (liveness from override register, not messages)
    () => [],
    // 7. mention events — none
    () => [],
  ]);

  // Complete projection: live register S=5,C=0,B=100 and frontier F=50.
  // F=50 ≤ B=100, S=5 > C=0 → isOverrideActive returns true.
  const overrides = new Map([[CHANNEL_ID, { s: 5, c: 0, b: 100 }]]);
  const frontiers = new Map([[CHANNEL_ID, 50]]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 500,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    getProjection: () => ({ loadComplete: true, frontiers, overrides }),
  });

  // Projection is the sole override authority when complete.  The fetched
  // tombstone does not suppress the live projection register — the rail lights.
  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchCommunityUnread adversarial-6: torn fetched register (ov_s+ov_c without ov_b) → rail stays dark", async () => {
  // Integration witness: a fetched read-state event carrying an illegal partial
  // override group — ov_s and ov_c present, ov_b ABSENT — must not light the
  // rail.  The parser drops incomplete groups (any shape other than all-three or
  // C-only tombstone), so the register never reaches liveness evaluation.
  //
  // This locks the parser → structured merge → rail path at the observer level.
  // The readStateOverride parser-matrix tests confirm the drop at the unit level;
  // this witness confirms the integration: a torn fetched group cannot influence
  // fetchCommunityUnread() rail output.
  const SLOT = "b".repeat(32); // valid 32-hex slot, distinct from other tests
  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events
    () => [],
    // 4. read-state: torn group — ov_s + ov_c present, ov_b absent (S+C shape).
    //    The parser treats any shape other than {S,C,B} or {C-only tombstone}
    //    as invalid and drops the entire group.  No override register reaches
    //    the authoritative map.  No message events follow either.
    () => [
      event({
        pubkey: PUBKEY,
        created_at: 300,
        tags: [
          ["d", `read-state:${SLOT}`],
          ["t", "read-state"],
        ],
        content: JSON.stringify({
          v: 1,
          client_id: "client",
          contexts: {
            [`ov_s:${CHANNEL_ID}`]: 5,
            [`ov_c:${CHANNEL_ID}`]: 0,
            // ov_b: intentionally absent → illegal S+C partial group, dropped
          },
        }),
      }),
    ],
    // 5. mutes
    () => [],
    // 6. unread events — none
    () => [],
    // 7. mention events — none
    () => [],
  ]);

  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 400,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // No getProjection — fetched-deduped is sole authority, torn group is dropped.
  });

  // Torn group is silently dropped by the parser; no override register present;
  // no message events — rail must remain dark.
  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchCommunityUnread adversarial-7: active fetched register, no complete projection → rail lights", async () => {
  // Positive fallback witness: when getProjection() is absent (null), the observer
  // must decode the fetched read-state events with mergeReadStateEventsStructured
  // and use the resulting `overrides` map as override authority. An active register
  // (S > C, B ≥ F) in the fetched state must light the rail.
  //
  // This locks the fallback branch: before the fix, the fallback set authoritative
  // to an empty Map(), so any remote NIP-RS override older than the seven-day
  // horizon would be silently ignored. Adversarial-1 covers the tombstone (dark)
  // case; this witness covers the live-register (lit) case.
  const SLOT = "c".repeat(32); // valid 32-hex slot, distinct from other tests

  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility events
    () => [],
    // 4. read-state: live register S=5,C=0,B=100; frontier F=50 (B ≥ F, S > C → active)
    () => [
      event({
        pubkey: PUBKEY,
        created_at: 300,
        tags: [
          ["d", `read-state:${SLOT}`],
          ["t", "read-state"],
        ],
        content: JSON.stringify({
          v: 1,
          client_id: "client",
          contexts: {
            [CHANNEL_ID]: 50,
            [`ov_s:${CHANNEL_ID}`]: 5,
            [`ov_c:${CHANNEL_ID}`]: 0,
            [`ov_b:${CHANNEL_ID}`]: 100,
          },
        }),
      }),
    ],
    // 5. mutes
    () => [],
    // 6. unread events — none (liveness from override register only)
    () => [],
    // 7. mention events — none
    () => [],
  ]);

  // No getProjection — fallback path must use fetched structured overrides.
  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 400,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // Intentionally no getProjection.
  });

  // Fetched active register (S=5 > C=0, B=100 ≥ F=50) must light the rail.
  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchCommunityUnread adversarial-8: fetched inactive register supersedes qualifying local forced-unread entry → rail stays dark", async () => {
  // Bites: communityUnreadObserver.ts:else if condition — local forcedUnreadMap
  // fallback runs ONLY when reg === undefined. An inactive fetched register (a
  // remote tombstone or F>B deactivation) plus a qualifying local entry must keep
  // the rail dark — the fetched verdict is final.
  //
  // Before the fix, the else-if ran when reg was absent OR inactive:
  //   } else if (Object.hasOwn(forcedUnreadMap, channel.id)) {
  // An inactive reg (reg !== undefined, isOverrideActive === false) fell into this
  // branch and could resurrect the override from stale local state. Fix: the
  // fallback runs ONLY when reg === undefined.
  //
  // Setup: fetched event carries a cleared register (S=5, C=8 → C>S, inactive).
  // Channel frontier = 50 (from fetched read-state).
  // Forced-unread entry has markerAtWhenForced=50 (would pass the frontier gate).
  // Expected: rail stays dark — fetched inactive verdict is final.
  const SLOT = "d".repeat(32); // valid 32-hex slot, distinct from other tests

  const relay = relayFor([
    // 1. member events
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["p", PUBKEY],
        ],
      }),
    ],
    // 2. metadata
    () => [
      event({
        tags: [
          ["d", CHANNEL_ID],
          ["t", "stream"],
        ],
      }),
    ],
    // 3. visibility
    () => [],
    // 4. read-state: cleared register S=5, C=8, B=100 (C>S → inactive); frontier=50
    () => [
      event({
        pubkey: PUBKEY,
        created_at: 300,
        tags: [
          ["d", `read-state:${SLOT}`],
          ["t", "read-state"],
        ],
        content: JSON.stringify({
          v: 1,
          client_id: "client",
          contexts: {
            [CHANNEL_ID]: 50, // frontier F=50
            [`ov_s:${CHANNEL_ID}`]: 5,
            [`ov_c:${CHANNEL_ID}`]: 8, // C=8 > S=5 → inactive
            [`ov_b:${CHANNEL_ID}`]: 100,
          },
        }),
      }),
    ],
    // 5. mutes
    () => [],
    // 6. unread events — none
    () => [],
    // 7. mention events — none
    () => [],
  ]);

  // Inject a qualifying forced-unread entry: markerAtWhenForced=50 (equals channel
  // frontier, so readAt <= markerAtWhenForced would pass the gate). Under the old
  // code this would resurrect the override; under the fix it is suppressed.
  const result = await fetchCommunityUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 400,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    readForcedUnread: () => ({
      [CHANNEL_ID]: { markerAtWhenForced: 50, sources: ["manual"] },
    }),
    // No getProjection — fallback path.
  });

  // Fetched inactive register is authoritative; local forced-unread must not
  // resurrect it. Rail stays dark.
  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});
