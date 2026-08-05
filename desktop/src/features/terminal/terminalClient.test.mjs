import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

const callbacks = new Map();
const calls = [];
let nextCallback = 1;
let attachDuringInvoke;

globalThis.isTauri = true;
globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke(command, args) {
      calls.push({ command, args });
      if (command === "terminal_attach") attachDuringInvoke?.(args);
      return Promise.resolve(
        command === "terminal_attach"
          ? {
              sessionId: "session-1",
              subscriptionId: "subscription-1",
              viewport: { columns: 80, generation: 0, screenLines: 24 },
            }
          : undefined,
      );
    },
    transformCallback(callback) {
      const id = nextCallback++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id) {
      callbacks.delete(id);
    },
  },
  isTauri: true,
};

const { TerminalConnection } = await import("./terminalClient.ts");

function emit(channel, message, index = 0) {
  const id = Number(channel.toJSON().slice("__CHANNEL__:".length));
  callbacks.get(id)({ index, message });
}

const request = {
  channelId: "channel-1",
  channelName: "general",
  npub: "npub1test",
  relayUrl: "wss://relay.example",
  columns: 80,
  rows: 24,
  pixelWidth: 800,
  pixelHeight: 408,
};

const frame = {
  type: "frame",
  payload: {
    subscriptionId: "subscription-1",
    sequence: 7,
    rows: [],
    cursor: { column: 0, line: 0, visible: true },
    full: true,
    viewport: { columns: 80, generation: 0, screenLines: 24 },
    bracketedPaste: true,
    focusReporting: true,
  },
};

afterEach(() => {
  calls.length = 0;
  attachDuringInvoke = undefined;
});

test("attach retains a bootstrap frame delivered before invoke resolves", async () => {
  const deliveries = [];
  attachDuringInvoke = ({ onFrame }) => emit(onFrame, frame);
  const connection = await TerminalConnection.attach(
    request,
    () => {},
    (delivery) => deliveries.push(delivery),
  );

  assert.equal(connection.sessionId, "session-1");
  assert.equal(deliveries.length, 1);
  assert.equal(deliveries[0].frame.sequence, 7);
  await deliveries[0].acknowledge();
  assert.deepEqual(calls.at(-1), {
    command: "terminal_ack",
    args: {
      sessionId: "session-1",
      subscriptionId: "subscription-1",
      sequence: 7,
    },
  });
});

test("stale subscription frames are ignored", async () => {
  const deliveries = [];
  let channel;
  attachDuringInvoke = ({ onFrame }) => {
    channel = onFrame;
  };
  await TerminalConnection.attach(
    request,
    () => {},
    (value) => deliveries.push(value),
  );
  emit(
    channel,
    { ...frame, payload: { ...frame.payload, subscriptionId: "stale" } },
    0,
  );
  assert.deepEqual(deliveries, []);
});
