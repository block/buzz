import assert from "node:assert/strict";
import test from "node:test";

import {
  buildReconnectReplayFilter,
  replayLiveSubscriptions,
} from "./relayReconnectReplay.ts";
import { RelayClient } from "./relayClientSession.ts";

function replayFilter(filter, since, until) {
  return buildReconnectReplayFilter(filter, since, until);
}

function event(id, createdAt) {
  return {
    id,
    pubkey: "pubkey",
    created_at: createdAt,
    kind: 9,
    tags: [],
    content: "",
    sig: "sig",
  };
}

function eventRange(prefix, start, count) {
  return Array.from({ length: count }, (_, index) =>
    event(`${prefix}-${index}`, start + index),
  );
}

test("reconnect replay preserves small steady-state limits when adding since", () => {
  const filter = {
    kinds: [9, 40002],
    "#h": ["channel-1"],
    limit: 50,
  };

  assert.deepEqual(replayFilter(filter, 123), {
    kinds: [9, 40002],
    "#h": ["channel-1"],
    limit: 50,
    since: 123,
  });
});

test("reconnect replay caps large steady-state limits", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 1000,
  };

  assert.deepEqual(replayFilter(filter, 123), {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 500,
    since: 123,
  });
});

test("reconnect replay keeps the stricter existing since window", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
    since: 200,
  };

  assert.deepEqual(replayFilter(filter, 123), {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
    since: 200,
  });
});

test("reconnect replay applies the stricter until window", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
    until: 300,
  };

  assert.deepEqual(replayFilter(filter, 123, 400), {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
    since: 123,
    until: 300,
  });
});

test("initial subscription replay preserves the original filter", () => {
  const filter = {
    kinds: [9],
    "#h": ["channel-1"],
    limit: 50,
  };

  assert.equal(replayFilter(filter, undefined), filter);
});

test("channel reconnect replay pages the missed window until a short page", async () => {
  const delivered = [];
  const historyFilters = [];
  const sentPayloads = [];
  const pages = [
    eventRange("newest", 1501, 500),
    eventRange("middle", 1002, 500),
    eventRange("oldest", 995, 8),
  ];
  const client = new RelayClient();
  const filter = client.buildChannelFilter("channel-1", 50);
  const subscriptions = new Map([
    [
      "live-1",
      {
        mode: "live",
        filter,
        onEvent: (event) => delivered.push(event),
        lastSeenCreatedAt: 1000,
      },
    ],
  ]);

  await replayLiveSubscriptions({
    subscriptions,
    now: 2000,
    sendRaw: async (payload) => {
      sentPayloads.push(payload);
    },
    requestHistory: async (filter) => {
      historyFilters.push(filter);
      return pages.shift() ?? [];
    },
  });

  assert.deepEqual(sentPayloads, [
    [
      "REQ",
      "live-1",
      {
        kinds: filter.kinds,
        "#h": ["channel-1"],
        limit: 50,
      },
    ],
  ]);
  assert.deepEqual(historyFilters, [
    {
      kinds: filter.kinds,
      "#h": ["channel-1"],
      limit: 500,
      since: 995,
      until: 2000,
    },
    {
      kinds: filter.kinds,
      "#h": ["channel-1"],
      limit: 500,
      since: 995,
      until: 1501,
    },
    {
      kinds: filter.kinds,
      "#h": ["channel-1"],
      limit: 500,
      since: 995,
      until: 1002,
    },
  ]);
  assert.equal(delivered.length, 1008);
});
