import assert from "node:assert/strict";
import test from "node:test";

import { ReadOnlyRelayClient } from "./readOnlyRelayClient.ts";

const AUTH_EVENT = {
  id: "observer-auth-event",
  pubkey: "observer-pubkey",
  created_at: 1,
  kind: 22242,
  tags: [],
  content: "",
  sig: "observer-signature",
};

test("connect preserves AUTH delivered before the socket ID resolves", async () => {
  const previousWindow = globalThis.window;
  let callbackId = 0;
  let messageChannel;
  const sentTypes = [];

  globalThis.window = {
    setTimeout: globalThis.setTimeout.bind(globalThis),
    clearTimeout: globalThis.clearTimeout.bind(globalThis),
    __TAURI_INTERNALS__: {
      transformCallback: () => ++callbackId,
      unregisterCallback: () => {},
      invoke: async (command, args) => {
        if (command === "plugin:websocket|connect") {
          messageChannel = args.onMessage;
          messageChannel.onmessage({
            type: "Text",
            data: JSON.stringify(["AUTH", "early-observer-challenge"]),
          });
          await new Promise((resolve) => globalThis.setTimeout(resolve, 10));
          return 7;
        }

        if (command === "create_auth_event") {
          assert.equal(args.challenge, "early-observer-challenge");
          assert.equal(args.relayUrl, "wss://observer.example");
          return JSON.stringify(AUTH_EVENT);
        }

        if (command === "plugin:websocket|send") {
          const [type, event] = JSON.parse(args.message.data);
          sentTypes.push(type);
          if (type === "AUTH") {
            messageChannel.onmessage({
              type: "Text",
              data: JSON.stringify(["OK", event.id, true, ""]),
            });
          }
          return;
        }

        if (command === "plugin:websocket|disconnect") return;
        throw new Error(`Unexpected Tauri command: ${command}`);
      },
    },
  };

  const client = new ReadOnlyRelayClient("wss://observer.example");
  const connection = client.connect();
  try {
    const connected = await Promise.race([
      connection.then(() => true),
      new Promise((resolve) =>
        globalThis.setTimeout(() => resolve(false), 250),
      ),
    ]);

    assert.equal(connected, true, "early AUTH must complete the connection");
    assert.deepEqual(sentTypes, ["AUTH"]);
  } finally {
    client.disconnect();
    await connection.catch(() => {});
    globalThis.window = previousWindow;
  }
});
