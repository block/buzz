import assert from "node:assert/strict";
import { after, test } from "node:test";

const previousWindow = globalThis.window;
const previousInternals = globalThis.__TAURI_INTERNALS__;
const writes = [];
let receive;
const internals = {
  transformCallback() {
    return 1;
  },
  async invoke(command, args) {
    if (command === "plugin:websocket|connect") {
      receive = args.onMessage.onmessage;
      queueMicrotask(() =>
        receive({ type: "Text", data: JSON.stringify(["AUTH", "challenge"]) }),
      );
      return 8;
    }
    if (command === "create_auth_event")
      return JSON.stringify({ id: "auth-event", kind: 22242 });
    if (command === "plugin:websocket|send") {
      const frame = JSON.parse(args.message.data);
      writes.push({ socket: args.id, frame });
      if (frame[0] === "AUTH")
        queueMicrotask(() =>
          receive({
            type: "Text",
            data: JSON.stringify(["OK", frame[1].id, true, ""]),
          }),
        );
      if (frame[0] === "REQ" && args.id === 8)
        queueMicrotask(() =>
          receive({ type: "Text", data: JSON.stringify(["EOSE", frame[1]]) }),
        );
      return;
    }
    if (command === "plugin:websocket|disconnect") return;
    assert.fail(`Unexpected IPC ${command}`);
  },
};
globalThis.window = {
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval,
  __TAURI_INTERNALS__: internals,
};
globalThis.__TAURI_INTERNALS__ = internals;
const { RelayClient } = await import("./relayClientSession.ts");
after(() => {
  globalThis.window = previousWindow;
  globalThis.__TAURI_INTERNALS__ = previousInternals;
});

const flush = async () => {
  for (let i = 0; i < 30; i++) await Promise.resolve();
};
test("employee reconnect replays mounted live subscriptions and preserves repair listeners", async () => {
  const client = new RelayClient();
  client.wsId = 7;
  client.relayUrl = "wss://employee.example";
  client.hasConnectedOnce = true;
  const events = [];
  let repairs = 0;
  client.subscribeToReconnects(() => repairs++);
  const pending = client.subscribeLive(
    { kinds: [0], authors: ["employee-key"] },
    (event) => events.push(event),
  );
  try {
    await flush();
    const id = writes.find((write) => write.frame[0] === "REQ").frame[1];
    await client.handleWsMessage(
      { type: "Text", data: JSON.stringify(["EOSE", id]) },
      client.connectionGeneration,
    );
    const unsubscribe = await pending;
    await client.reconnect();
    assert.ok(
      writes.some(
        (write) =>
          write.socket === 8 &&
          write.frame[0] === "REQ" &&
          write.frame[1] === id,
      ),
    );
    assert.equal(repairs, 1);
    receive({
      type: "Text",
      data: JSON.stringify([
        "EVENT",
        id,
        {
          id: "new-event",
          kind: 0,
          pubkey: "employee-key",
          created_at: 1,
          content: "{}",
          tags: [],
        },
      ]),
    });
    await flush();
    // Delivery is batched by the same production path used before reconnect.
    await new Promise((resolve) => setTimeout(resolve, 60));
    assert.deepEqual(
      events.map((event) => event.id),
      ["new-event"],
    );
    unsubscribe();
  } finally {
    client.disconnect();
  }
});
