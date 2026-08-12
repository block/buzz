import assert from "node:assert/strict";
import test from "node:test";

const RELAY_URL = "wss://relay.example/";
const AUTH_EVENT_ID = "aa".repeat(32);
const AUTHOR = "bb".repeat(32);
const calls = [];
const callbacks = new Map();
let nextCallbackId = 1;
let messageChannel;
let projectionChannel;
let socketId = 4_242;
let authDelivery = "early";
let authChallengeCopies = 1;

globalThis.isTauri = true;
globalThis.window = globalThis;
globalThis.window.__TAURI_INTERNALS__ = {
  invoke(command, args) {
    calls.push({ command, args });
    switch (command) {
      case "get_relay_ws_url":
        return Promise.resolve(RELAY_URL);
      case "plugin:websocket|connect_with_status": {
        messageChannel = args.onMessage;
        projectionChannel = args.onProjection;
        projectionChannel.onmessage({
          eventAuthorPubkey: AUTHOR,
          freshUntil: Math.floor(Date.now() / 1_000) + 60,
        });
        const deliverAuth = () =>
          messageChannel.onmessage({
            type: "Text",
            data: JSON.stringify(["AUTH", `${authDelivery}-challenge`]),
          });
        const deliverAuthCopies = () => {
          for (let index = 0; index < authChallengeCopies; index += 1) {
            deliverAuth();
          }
        };
        if (authDelivery === "early") deliverAuthCopies();
        else window.setTimeout(deliverAuthCopies, 0);
        return Promise.resolve(socketId);
      }
      case "create_auth_event":
        return Promise.resolve(
          JSON.stringify({
            content: "",
            created_at: 1,
            id: AUTH_EVENT_ID,
            kind: 22_242,
            pubkey: AUTHOR,
            sig: "cc".repeat(64),
            tags: [],
          }),
        );
      case "plugin:websocket|send": {
        const payload = JSON.parse(args.message.data);
        if (payload[0] === "AUTH") {
          queueMicrotask(() => {
            messageChannel.onmessage({
              type: "Text",
              data: JSON.stringify(["OK", AUTH_EVENT_ID, true, ""]),
            });
          });
        }
        return Promise.resolve();
      }
      case "plugin:websocket|disconnect":
        return Promise.resolve();
      default:
        throw new Error(`Unexpected Tauri command: ${command}`);
    }
  },
  transformCallback(callback) {
    const id = nextCallbackId++;
    callbacks.set(id, callback);
    return id;
  },
  unregisterCallback(id) {
    callbacks.delete(id);
  },
};

const { RelayClient } = await import("./relayClientSession.ts");
const { RelayClientStatusConnection } = await import(
  "./relayClientStatusConnection.ts"
);
const projectionStore = await import(
  "@/features/binding-status/currentProjectionStore.ts"
);

test("primary status connection binds early AUTH and projection to its native socket", async () => {
  const client = new RelayClient();
  await client.preconnect();

  const connectCalls = calls.filter(({ command }) =>
    command.startsWith("plugin:websocket|connect"),
  );
  assert.equal(connectCalls.length, 1);
  assert.equal(connectCalls[0].command, "plugin:websocket|connect_with_status");
  assert.equal(connectCalls[0].args.url, RELAY_URL);
  assert.equal(
    projectionStore.getCurrentProjectionSnapshot(),
    null,
    "projection delivered before the native id is bound stays fenced",
  );

  const authCall = calls.find(({ command }) => command === "create_auth_event");
  assert.deepEqual(authCall?.args, {
    challenge: "early-challenge",
    nativeWebsocketId: socketId,
    relayUrl: RELAY_URL,
  });

  const current = {
    eventAuthorPubkey: AUTHOR,
    freshUntil: Math.floor(Date.now() / 1_000) + 60,
  };
  projectionChannel.onmessage(current);
  assert.deepEqual(projectionStore.getCurrentProjectionSnapshot(), current);

  client.disconnect();
  assert.equal(projectionStore.getCurrentProjectionSnapshot(), null);
  projectionChannel.onmessage(current);
  assert.equal(
    projectionStore.getCurrentProjectionSnapshot(),
    null,
    "a retired connection channel cannot repopulate the store",
  );
});

test("late retirement of connection A cannot clear replacement B", () => {
  const active = new Set([8_001, 8_002]);
  const makeConnection = () =>
    new RelayClientStatusConnection(
      (id) => active.has(id),
      (id) => active.has(id),
      () => {},
      async () => {},
    );
  const currentFor = (author) => ({
    eventAuthorPubkey: author,
    freshUntil: Math.floor(Date.now() / 1_000) + 60,
  });

  const connectionA = makeConnection();
  connectionA.bind(8_001, RELAY_URL);
  connectionA.projectionChannel.onmessage(currentFor(AUTHOR));

  const connectionB = makeConnection();
  connectionB.bind(8_002, RELAY_URL);
  const authorB = "ee".repeat(32);
  connectionB.projectionChannel.onmessage(currentFor(authorB));
  assert.equal(
    projectionStore.getCurrentProjectionSnapshot()?.eventAuthorPubkey,
    authorB,
  );

  active.delete(8_001);
  connectionA.retire();
  assert.equal(
    projectionStore.getCurrentProjectionSnapshot()?.eventAuthorPubkey,
    authorB,
    "late A retirement is owner-fenced",
  );
  connectionB.retire();
  assert.equal(projectionStore.getCurrentProjectionSnapshot(), null);
});

test("duplicate AUTH challenges are handled once per native connection", async () => {
  calls.length = 0;
  authDelivery = "early";
  authChallengeCopies = 2;
  socketId = 6_666;

  const client = new RelayClient();
  await client.preconnect();

  assert.equal(
    calls.filter(({ command }) => command === "create_auth_event").length,
    1,
  );
  assert.equal(
    calls.filter(({ command, args }) => {
      if (command !== "plugin:websocket|send") return false;
      return JSON.parse(args.message.data)[0] === "AUTH";
    }).length,
    1,
  );

  authChallengeCopies = 1;
  client.disconnect();
});

test("AUTH delivered after connect uses the returned native socket id", async () => {
  calls.length = 0;
  authDelivery = "normal";
  socketId = 7_777;

  const client = new RelayClient();
  await client.preconnect();

  const authCall = calls.find(({ command }) => command === "create_auth_event");
  assert.deepEqual(authCall?.args, {
    challenge: "normal-challenge",
    nativeWebsocketId: socketId,
    relayUrl: RELAY_URL,
  });
  client.disconnect();
});

test("ordinary AUTH still succeeds without browser-visible status scope", async () => {
  calls.length = 0;
  authDelivery = "normal";
  socketId = 8_888;

  const client = new RelayClient();
  await client.preconnect();

  const authSend = calls.find(({ command, args }) => {
    if (command !== "plugin:websocket|send") return false;
    return JSON.parse(args.message.data)[0] === "AUTH";
  });
  assert.equal(authSend?.args.id, socketId);
  assert.deepEqual(JSON.parse(authSend.args.message.data)[1].tags, []);

  const malformed = {
    connectionEpoch: "11111111-1111-4111-8111-111111111111",
    eventAuthorPubkey: AUTHOR,
    freshUntil: Math.floor(Date.now() / 1_000) + 60,
  };
  projectionChannel.onmessage(malformed);
  assert.equal(
    projectionStore.getCurrentProjectionSnapshot(),
    null,
    "connectionEpoch is rejected as an unknown IPC key",
  );

  client.disconnect();
});
