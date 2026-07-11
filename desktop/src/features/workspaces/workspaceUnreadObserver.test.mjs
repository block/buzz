import assert from "node:assert/strict";
import test from "node:test";

import {
  extractHiddenDmIds,
  extractMemberChannelIds,
  fetchWorkspaceUnread,
  resolveObservedChannels,
} from "./workspaceUnreadObserver.ts";

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

test("fetchWorkspaceUnread returns dot and mention count without total unread count", async () => {
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

  const result = await fetchWorkspaceUnread({
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

test("fetchWorkspaceUnread ignores self-authored and read thread/message events", async () => {
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
          ["d", "read-state:test"],
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

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async (value) => value,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchWorkspaceUnread excludes muted-only channel — returns hasUnread:false mentionCount:0", async () => {
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

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async (value) => value,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchWorkspaceUnread counts unmuted channel but skips muted channel", async () => {
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

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
    decryptMutes: async (value) => value,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 1 });
});

test("fetchWorkspaceUnread treats decryption failure as empty mutes set", async () => {
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

  const result = await fetchWorkspaceUnread({
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

test("fetchWorkspaceUnread treats absent mutes blob as empty mutes set", async () => {
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

  const result = await fetchWorkspaceUnread({
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

test("fetchWorkspaceUnread threaded reply in untracked root → hasUnread:false", async () => {
  const relay = baseRelay(threadedReplyEvent());

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    // No root in any set → gate rejects the threaded reply
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchWorkspaceUnread threaded reply in participatedRootIds → hasUnread:true", async () => {
  const relay = baseRelay(threadedReplyEvent());

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships({
      participatedRootIds: new Set([THREAD_ROOT_2]),
    }),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchWorkspaceUnread #p-mention reply in untracked root → hasUnread:true (mention overrides)", async () => {
  // A @mention of the user bypasses the follow/participation gate
  const relay = baseRelay(
    threadedReplyEvent({
      id: "mention-reply".padEnd(64, "0"),
      extraTags: [["p", PUBKEY]],
    }),
  );

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchWorkspaceUnread top-level post → hasUnread:true (no thread gate)", async () => {
  // Top-level posts have no parentId — shouldNotifyForEvent returns true
  const relay = baseRelay(
    event({
      id: "toplevel".padEnd(64, "0"),
      created_at: 20,
      tags: [["h", CHANNEL_ID]],
    }),
  );

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchWorkspaceUnread threaded reply whose root is in mutedRootIds → hasUnread:false", async () => {
  const relay = baseRelay(
    threadedReplyEvent({ id: "muted-reply".padEnd(64, "0") }),
  );

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    // Root is participated but also muted — mute wins
    readThreadRelationships: readRelationships({
      participatedRootIds: new Set([THREAD_ROOT_2]),
      mutedRootIds: new Set([THREAD_ROOT_2]),
    }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

// ── Forced-unread persistence gate tests ─────────────────────────────────

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
          ["d", "read-state:test"],
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

test("fetchWorkspaceUnread forced-unread channel lights rail dot (hasUnread:true)", async () => {
  const relay = quietRelay();

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // Forced-unread map: CHANNEL_ID forced when marker was null (no prior read)
    readForcedUnread: () => ({ [CHANNEL_ID]: null }),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchWorkspaceUnread forced-unread channel not in member list → hasUnread:false", async () => {
  // The channel is marked forced-unread but the user is not a member of ANY
  // channel — forced-unread is not consulted when the channel set is empty.
  const relay = relayFor([
    // 1. member events — empty
    () => [],
  ]);

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    readForcedUnread: () => ({ [CHANNEL_ID]: null }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchWorkspaceUnread forced-unread channel that is also muted → hasUnread:false", async () => {
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
    // 4. read-state (parallel with mutes)
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

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // CHANNEL_ID is both forced-unread AND muted — mute wins
    readForcedUnread: () => ({ [CHANNEL_ID]: null }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchWorkspaceUnread readForcedUnread returns empty map → falls through to relay gate", async () => {
  // No forced-unread, but there IS a real unread event → hasUnread:true via relay
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
    // 5. mutes
    () => [],
    // 6. unread events
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

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    readForcedUnread: () => ({}), // empty — no forced-unread
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchWorkspaceUnread forced-unread + synced marker advanced PAST baseline → hasUnread:false", async () => {
  // markerAtWhenForced = 50, observed readAt = 100 (100 > 50 → cross-device read wins)
  const relay = quietRelayWithReadState(100);

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 200,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // Forced at marker=50; synced marker is now 100 — covers the force
    readForcedUnread: () => ({ [CHANNEL_ID]: 50 }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});

test("fetchWorkspaceUnread forced-unread + synced marker NOT advanced past baseline → hasUnread:true", async () => {
  // markerAtWhenForced = 100, observed readAt = 100 (equal → force still stands)
  const relay = quietRelayWithReadState(100);

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 200,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // Forced at marker=100; synced marker is still 100 — no newer read
    readForcedUnread: () => ({ [CHANNEL_ID]: 100 }),
  });

  assert.deepEqual(result, { hasUnread: true, mentionCount: 0 });
});

test("fetchWorkspaceUnread forced-unread with null baseline + synced marker present → hasUnread:false", async () => {
  // markerAtWhenForced = null (no marker at force-time), but readAt = 50 now
  // A cross-device read appeared after the force → do NOT light the dot
  const relay = quietRelayWithReadState(50);

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 200,
    decryptReadState: async (v) => v,
    decryptMutes: async (v) => v,
    readThreadRelationships: readRelationships(),
    // Forced when no marker existed; a cross-device read has since appeared
    readForcedUnread: () => ({ [CHANNEL_ID]: null }),
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});
