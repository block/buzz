import assert from "node:assert/strict";
import test, { beforeEach } from "node:test";

import {
  chunkChannelContexts,
  publishWorkspaceReadState,
} from "./workspaceMarkRead.ts";

const PUBKEY = "a".repeat(64);
const READ_AT = 1_700_000_000;

function makeLocalStorage() {
  const store = new Map();
  return {
    get length() {
      return store.size;
    },
    key: (i) => [...store.keys()][i] ?? null,
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, String(value)),
    removeItem: (key) => store.delete(key),
  };
}

beforeEach(() => {
  globalThis.localStorage = makeLocalStorage();
  globalThis.window = { localStorage: globalThis.localStorage };
});

test("chunkChannelContexts puts all channels in one blob when they fit", () => {
  const chunks = chunkChannelContexts(["chan-1", "chan-2"], READ_AT, "client");
  assert.equal(chunks.length, 1);
  assert.deepEqual(chunks[0], { "chan-1": READ_AT, "chan-2": READ_AT });
});

test("chunkChannelContexts splits when the blob exceeds the byte budget", () => {
  const ids = Array.from({ length: 40 }, (_, i) => `channel-${i}`);
  const chunks = chunkChannelContexts(ids, READ_AT, "client", 512);
  assert.ok(chunks.length > 1);
  const merged = Object.assign({}, ...chunks);
  assert.equal(Object.keys(merged).length, ids.length);
  const encoder = new TextEncoder();
  for (const contexts of chunks) {
    const bytes = encoder.encode(
      JSON.stringify({ v: 1, client_id: "client", contexts }),
    ).length;
    assert.ok(bytes <= 512, `chunk exceeds budget: ${bytes}`);
  }
});

test("chunkChannelContexts drops overflow beyond maxSlots instead of flooding", () => {
  const ids = Array.from({ length: 100 }, (_, i) => `channel-${i}`);
  const chunks = chunkChannelContexts(ids, READ_AT, "client", 128, 2);
  assert.equal(chunks.length, 2);
});

function membersEvent(channelIds) {
  return {
    id: "e".repeat(64),
    pubkey: "relay",
    created_at: 1,
    kind: 39002,
    tags: channelIds.map((id) => ["d", id]),
    content: "",
    sig: "sig",
  };
}

function metadataEvent(channelId, extraTags = []) {
  return {
    id: "f".repeat(64),
    pubkey: "relay",
    created_at: 1,
    kind: 39000,
    tags: [["d", channelId], ...extraTags],
    content: "",
    sig: "sig",
  };
}

function makeClient({ channelIds, metadata }) {
  const published = [];
  return {
    published,
    async fetchEvents(filter) {
      if (filter.kinds?.includes(39002)) return [membersEvent(channelIds)];
      if (filter.kinds?.includes(39000)) return metadata;
      return [];
    },
    async publishEvent(event) {
      published.push(event);
    },
  };
}

test("publishWorkspaceReadState publishes one read-state event covering observed channels", async () => {
  const client = makeClient({
    channelIds: ["chan-1", "chan-2"],
    metadata: [metadataEvent("chan-1"), metadataEvent("chan-2")],
  });
  const encrypted = [];

  await publishWorkspaceReadState({
    client,
    pubkey: PUBKEY,
    relayUrl: "wss://relay.example",
    nowSeconds: READ_AT,
    encrypt: async (plaintext) => {
      encrypted.push(plaintext);
      return `cipher:${encrypted.length}`;
    },
    sign: async (input) => ({
      id: `signed-${encrypted.length}`,
      pubkey: PUBKEY,
      created_at: input.createdAt,
      kind: input.kind,
      tags: input.tags,
      content: input.content,
      sig: "sig",
    }),
  });

  assert.equal(client.published.length, 1);
  const event = client.published[0];
  assert.equal(event.kind, 30078);
  assert.equal(event.created_at, READ_AT);
  assert.ok(
    event.tags.some(
      (tag) => tag[0] === "d" && tag[1].startsWith("read-state:"),
    ),
  );
  assert.ok(
    event.tags.some((tag) => tag[0] === "t" && tag[1] === "read-state"),
  );

  const blob = JSON.parse(encrypted[0]);
  assert.equal(blob.v, 1);
  assert.deepEqual(blob.contexts, { "chan-1": READ_AT, "chan-2": READ_AT });
});

test("publishWorkspaceReadState skips archived channels and publishes nothing when none remain", async () => {
  const client = makeClient({
    channelIds: ["chan-1"],
    metadata: [metadataEvent("chan-1", [["archived", "true"]])],
  });

  await publishWorkspaceReadState({
    client,
    pubkey: PUBKEY,
    relayUrl: "wss://relay.example",
    nowSeconds: READ_AT,
    encrypt: async (plaintext) => plaintext,
    sign: async () => {
      throw new Error("must not sign when there is nothing to publish");
    },
  });

  assert.equal(client.published.length, 0);
});

test("publishWorkspaceReadState reuses stable slot ids so blobs are replaceable", async () => {
  const makeArgs = (client) => ({
    client,
    pubkey: PUBKEY,
    relayUrl: "wss://relay.example",
    nowSeconds: READ_AT,
    encrypt: async (plaintext) => plaintext,
    sign: async (input) => ({
      id: Math.random().toString(),
      pubkey: PUBKEY,
      created_at: input.createdAt,
      kind: input.kind,
      tags: input.tags,
      content: input.content,
      sig: "sig",
    }),
  });

  const clientA = makeClient({
    channelIds: ["chan-1"],
    metadata: [metadataEvent("chan-1")],
  });
  await publishWorkspaceReadState(makeArgs(clientA));
  const clientB = makeClient({
    channelIds: ["chan-1"],
    metadata: [metadataEvent("chan-1")],
  });
  await publishWorkspaceReadState(makeArgs(clientB));

  const dTag = (event) => event.tags.find((tag) => tag[0] === "d")[1];
  assert.equal(dTag(clientA.published[0]), dTag(clientB.published[0]));
});
