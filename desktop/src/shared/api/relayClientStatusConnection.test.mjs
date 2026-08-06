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
        // Native channels can deliver before the invoke response reaches JS.
        // A projection is not eligible until this exact connection is bound.
        projectionChannel.onmessage({
          connectionEpoch: "too-early",
          eventAuthorPubkey: AUTHOR,
          freshUntil: Math.floor(Date.now() / 1_000) + 60,
        });
        const deliverAuth = () =>
          messageChannel.onmessage({
            type: "Text",
            data: JSON.stringify(["AUTH", `${authDelivery}-challenge`]),
          });
        if (authDelivery === "early") deliverAuth();
        else window.setTimeout(deliverAuth, 0);
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
  assert.equal(connectCalls[0].args.onMessage, messageChannel);
  assert.equal(connectCalls[0].args.onProjection, projectionChannel);
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
  const authSend = calls.find(({ command, args }) => {
    if (command !== "plugin:websocket|send") return false;
    return JSON.parse(args.message.data)[0] === "AUTH";
  });
  assert.equal(authSend?.args.id, socketId);

  const current = {
    connectionEpoch: "opaque-native-epoch",
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
  assert.equal(
    "applyCurrentProjectionFromNative" in projectionStore,
    false,
    "the singleton has no browser-callable direct population fixture",
  );
});

test("AUTH delivered after connect uses the same returned native socket id", async () => {
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
  const authSend = calls.find(({ command, args }) => {
    if (command !== "plugin:websocket|send") return false;
    return JSON.parse(args.message.data)[0] === "AUTH";
  });
  assert.equal(authSend?.args.id, socketId);
  client.disconnect();
});
