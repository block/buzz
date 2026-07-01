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

test("extractMemberChannelIds deduplicates d tags", () => {
  assert.deepEqual(
    extractMemberChannelIds([
      event({ tags: [["d", "one"], ["d", "two"]] }),
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
        event({ created_at: 1, tags: [["d", "dm"], ["t", "stream"]] }),
        event({ created_at: 2, tags: [["d", "dm"], ["t", "dm"]] }),
        event({ tags: [["d", "stream"], ["archived", "true"]] }),
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
      event({ created_at: 2, tags: [["h", "new"], ["h", "other"]] }),
    ]),
    new Set(["new", "other"]),
  );
});

test("fetchWorkspaceUnread returns dot and mention count without total unread count", async () => {
  const relay = relayFor([
    () => [event({ tags: [["d", CHANNEL_ID], ["p", PUBKEY]] })],
    () => [event({ tags: [["d", CHANNEL_ID], ["t", "stream"]] })],
    () => [],
    () => [],
    () => [
      event({ id: "unread".padEnd(64, "0"), created_at: 20, tags: [["h", CHANNEL_ID]] }),
    ],
    () => [
      event({
        id: "mention".padEnd(64, "0"),
        created_at: 30,
        tags: [["h", CHANNEL_ID], ["p", PUBKEY]],
      }),
    ],
  ]);

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
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
    tags: [["h", CHANNEL_ID], ["p", PUBKEY]],
  });

  const relay = relayFor([
    () => [event({ tags: [["d", CHANNEL_ID], ["p", PUBKEY]] })],
    () => [event({ tags: [["d", CHANNEL_ID], ["t", "stream"]] })],
    () => [],
    () => [
      event({
        pubkey: PUBKEY,
        created_at: 80,
        tags: [["d", "read-state:test"], ["t", "read-state"]],
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
    () => [threadReply, selfMention],
    () => [threadReply, selfMention],
  ]);

  const result = await fetchWorkspaceUnread({
    client: relay,
    pubkey: PUBKEY,
    nowSeconds: 100,
    decryptReadState: async (value) => value,
  });

  assert.deepEqual(result, { hasUnread: false, mentionCount: 0 });
});
