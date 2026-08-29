import assert from "node:assert/strict";
import test from "node:test";

import { RelayClient } from "./relayClientSession.ts";
import { CHANNEL_EVENT_KINDS } from "@/shared/constants/kinds";
import {
  activateRateLimit,
  isRateLimited,
  resetRateLimitGate,
} from "./relayRateLimitGate.ts";

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
async function settle() {
  for (let i = 0; i < 12; i++)
    await new Promise((resolve) => setImmediate(resolve));
}

// Real RelayClient, AUTH policy, inbound channel dispatch, replay and publish.
// Only the Tauri IPC boundary is fake; do not stub ensureConnected or replay.
function harness({ autoAuth = true, repair = async () => [], sendReq } = {}) {
  const oldWindow = globalThis.window;
  const timers = new Set();
  const intervals = new Set();
  const sockets = new Map();
  const frames = [];
  const repairs = [];
  const closes = [];
  let sequence = 0;
  let activeSocket = 0;
  const emit = (id, frame) =>
    sockets.get(id).onmessage({
      type: "Text",
      data: JSON.stringify(frame),
    });
  const makeEvent = (input) => ({
    id: (++sequence).toString(16).padStart(64, "0"),
    pubkey: "a".repeat(64),
    created_at: Math.floor(Date.now() / 1000),
    sig: "",
    ...input,
  });
  globalThis.window = {
    setTimeout(fn, ms) {
      const timer = setTimeout(() => {
        timers.delete(timer);
        fn();
      }, ms);
      timers.add(timer);
      return timer;
    },
    clearTimeout(timer) {
      timers.delete(timer);
      clearTimeout(timer);
    },
    setInterval(fn, ms) {
      const timer = setInterval(fn, ms);
      intervals.add(timer);
      return timer;
    },
    clearInterval(timer) {
      intervals.delete(timer);
      clearInterval(timer);
    },
    __TAURI_INTERNALS__: {
      transformCallback: () => ++sequence,
      async invoke(command, args) {
        switch (command) {
          case "get_relay_ws_url":
            return "ws://fixture.invalid";
          case "plugin:websocket|connect": {
            const id = ++activeSocket;
            sockets.set(id, args.onMessage);
            emit(id, ["AUTH", `challenge-${id}`]);
            return id;
          }
          case "create_auth_event":
            return JSON.stringify(
              makeEvent({ kind: 22242, content: "", tags: [] }),
            );
          case "sign_event":
            return JSON.stringify(makeEvent(args));
          case "plugin:websocket|send": {
            const frame = JSON.parse(args.message.data);
            frames.push({ socket: args.id, frame });
            if (frame[0] === "AUTH" && autoAuth)
              emit(args.id, ["OK", frame[1].id, true, ""]);
            if (frame[0] === "EVENT")
              emit(args.id, ["OK", frame[1].id, true, ""]);
            if (frame[0] === "REQ") {
              await sendReq?.(args.id, frame);
              emit(args.id, ["EOSE", frame[1]]);
            }
            return;
          }
          case "get_channel_reconnect_repair":
            repairs.push(args);
            return repair(args);
          case "plugin:websocket|disconnect":
            closes.push(args.id);
            return;
          default:
            throw new Error(`unmocked IPC: ${command}`);
        }
      },
    },
  };
  resetRateLimitGate();
  const client = new RelayClient();
  return {
    client,
    frames,
    repairs,
    closes,
    intervals,
    emit,
    makeEvent,
    get activeSocket() {
      return activeSocket;
    },
    async seedChannel(channelId = "channel-a") {
      const dispose = await client.subscribeLive(
        {
          kinds: [...CHANNEL_EVENT_KINDS],
          "#h": [channelId],
          limit: 1000,
          since: Math.floor(Date.now() / 1000) - 10,
        },
        () => {},
      );
      const req = frames.findLast(({ frame }) => frame[0] === "REQ");
      emit(activeSocket, [
        "EVENT",
        req.frame[1],
        makeEvent({
          kind: 9,
          content: "seed",
          tags: [["h", channelId]],
        }),
      ]);
      await settle();
      return dispose;
    },
    async drop() {
      sockets
        .get(activeSocket)
        .onmessage({ type: "Close", data: { code: 1006, reason: "fixture" } });
      await settle();
    },
    restore() {
      client.disconnect();
      for (const timer of timers) clearTimeout(timer);
      for (const timer of intervals) clearInterval(timer);
      resetRateLimitGate();
      globalThis.window = oldWindow;
    },
  };
}

test("auth-ready connection and plain send do not await held reconnect history repair", async () => {
  const held = deferred();
  const h = harness({ repair: () => held.promise });
  const outcomes = [];
  try {
    await h.seedChannel();
    await h.drop();
    let reconnected = false;
    let sent;
    let reconnectSignals = 0;
    h.client.subscribeToReconnects(() => reconnectSignals++);
    outcomes.push(
      h.client.preconnect().then(() => {
        reconnected = true;
      }),
    );
    outcomes.push(
      h.client.sendMessage("channel-a", "send during repair").then((event) => {
        sent = event;
      }),
    );
    await settle();
    assert.equal(
      h.repairs.length,
      1,
      "test must hold an actual repair request",
    );
    assert.equal(h.client.getConnectionState(), "connected");
    assert.equal(
      reconnected,
      true,
      "preconnect resolves at AUTH, not at history completion",
    );
    assert.equal(reconnectSignals, 1);
    assert.ok(
      sent,
      "plain TS send reaches EVENT/OK while repair remains pending",
    );
    assert.ok(
      h.frames.some(
        ({ socket, frame }) => socket === 2 && frame[0] === "EVENT",
      ),
    );
    assert.equal(
      h.intervals.size,
      1,
      "watchdog starts while history is still pending",
    );
  } finally {
    held.resolve([]);
    await Promise.allSettled(outcomes);
    await settle();
    h.restore();
  }
});

test("no ready state, subscriptions or publish escapes before AUTH acknowledgement", async () => {
  const h = harness({ autoAuth: false });
  const outcomes = [];
  try {
    let ready = false;
    outcomes.push(
      h.client.preconnect().then(() => {
        ready = true;
      }),
    );
    outcomes.push(h.client.sendMessage("channel-a", "after auth"));
    outcomes.push(h.client.subscribeToChannelLive("channel-a", () => {}));
    await settle();
    assert.equal(ready, false);
    assert.equal(h.client.getConnectionState(), "connecting");
    assert.deepEqual(
      h.frames.map(({ frame }) => frame[0]),
      ["AUTH"],
    );
    const auth = h.frames[0].frame[1];
    h.emit(1, ["OK", auth.id, true, ""]);
    await Promise.all(outcomes);
    assert.equal(ready, true);
    assert.equal(h.client.getConnectionState(), "connected");
    assert.equal(
      h.frames.filter(({ frame }) => frame[0] === "EVENT").length,
      1,
    );
  } finally {
    h.restore();
  }
});

test("permanent AUTH rejection rejects send without starting replay", async () => {
  const h = harness({ autoAuth: false });
  try {
    const result = h.client
      .sendMessage("channel-a", "must not send")
      .catch((error) => error);
    await settle();
    h.emit(1, [
      "OK",
      h.frames[0].frame[1].id,
      false,
      "restricted: not a member",
    ]);
    assert.match((await result).message, /restricted/);
    assert.equal(h.client.getConnectionState(), "disconnected");
    assert.equal(h.repairs.length, 0);
    assert.deepEqual(
      h.frames.map(({ frame }) => frame[0]),
      ["AUTH"],
    );
  } finally {
    h.restore();
  }
});

test("stale background replay rejection cannot reset a replacement community socket", async () => {
  const heldReq = deferred();
  const h = harness({
    sendReq: (socket) => (socket === 2 ? heldReq.promise : undefined),
  });
  let oldConnect;
  try {
    await h.seedChannel();
    await h.drop();
    oldConnect = h.client.preconnect().catch((error) => error);
    await settle();
    assert.ok(
      h.frames.some(({ socket, frame }) => socket === 2 && frame[0] === "REQ"),
    );
    h.client.disconnect();
    await h.client.preconnect();
    assert.equal(h.activeSocket, 3);
    heldReq.reject(new Error("old socket write failed"));
    await oldConnect;
    await settle();
    assert.equal(h.client.getConnectionState(), "connected");
    assert.equal(
      h.closes.includes(3),
      false,
      "stale failure must not close the new socket",
    );
    assert.equal(h.intervals.size, 1);
    const event = await h.client.sendMessage(
      "community-b-channel",
      "new scope",
    );
    assert.equal(event.content, "new scope");
  } finally {
    heldReq.resolve();
    await oldConnect;
    await settle();
    h.restore();
  }
});

test("current background replay write failure still resets and schedules recovery", async () => {
  const heldReq = deferred();
  const h = harness({
    sendReq: (socket) => (socket === 2 ? heldReq.promise : undefined),
  });
  let connection;
  try {
    await h.seedChannel();
    await h.drop();
    connection = h.client.preconnect().catch((error) => error);
    await settle();
    heldReq.reject(new Error("current socket write failed"));
    await connection;
    await settle();
    assert.equal(h.client.getConnectionState(), "reconnecting");
    assert.ok(h.closes.includes(2));
    assert.equal(h.intervals.size, 0);
  } finally {
    heldReq.resolve();
    await connection;
    await settle();
    h.restore();
  }
});

test("auth readiness does not let new live REQs bypass a held replay gate", async () => {
  const h = harness();
  let subscription;
  try {
    await h.seedChannel();
    await h.drop();
    const before = h.frames.length;
    activateRateLimit(30);
    await h.client.preconnect();
    let ready = false;
    subscription = h.client
      .subscribeToChannelLive("channel-b", () => {})
      .then((dispose) => {
        ready = true;
        return dispose;
      });
    await settle();
    assert.equal(h.client.getConnectionState(), "connected");
    assert.equal(isRateLimited(), true);
    assert.equal(ready, false);
    assert.deepEqual(
      h.frames.slice(before).map(({ frame }) => frame[0]),
      ["AUTH"],
    );
    resetRateLimitGate();
    await subscription;
    await settle();
    const requests = h.frames
      .slice(before)
      .filter(({ frame }) => frame[0] === "REQ");
    assert.equal(
      requests.length,
      2,
      "one old replay and one new subscription, no duplicate REQ",
    );
    assert.equal(new Set(requests.map(({ frame }) => frame[1])).size, 2);
  } finally {
    resetRateLimitGate();
    await subscription?.then((dispose) => dispose());
    h.restore();
  }
});

test("community switch cancels a live subscription waiting for admission", async () => {
  const h = harness();
  let subscription;
  try {
    await h.client.preconnect();
    activateRateLimit(30);
    let outcome;
    subscription = h.client
      .subscribeToChannelLive("old-community", () => {})
      .then(
        (dispose) => {
          outcome = dispose;
        },
        (error) => {
          outcome = error;
        },
      );
    await settle();
    assert.equal(outcome, undefined, "subscription is waiting at admission");
    h.client.disconnect();
    await h.client.preconnect();
    resetRateLimitGate();
    await subscription;
    await settle();
    assert.ok(
      outcome instanceof Error,
      "old scope must be rejected, not installed on the new socket",
    );
    assert.equal(h.frames.filter(({ frame }) => frame[0] === "REQ").length, 0);
    assert.equal(h.client.getConnectionState(), "connected");
  } finally {
    resetRateLimitGate();
    await subscription;
    h.restore();
  }
});

test("replay snapshots old subscriptions before a gate's delayed timer fires", async () => {
  const h = harness();
  const now = Date.now;
  let dispose;
  try {
    await h.seedChannel();
    await h.drop();
    activateRateLimit(30);
    await h.client.preconnect();
    // A throttled webview timer may lag its wall-clock expiry. New callers see
    // an expired gate while the old replay still awaits that timer's promise.
    const afterExpiry = now() + 31_000;
    Date.now = () => afterExpiry;
    dispose = await h.client.subscribeToChannelLive("channel-b", () => {});
    resetRateLimitGate();
    await settle();
    const requests = h.frames.filter(
      ({ socket, frame }) => socket === 2 && frame[0] === "REQ",
    );
    assert.equal(
      requests.length,
      2,
      "new connection's subscription must not be replayed again",
    );
    assert.equal(new Set(requests.map(({ frame }) => frame[1])).size, 2);
  } finally {
    Date.now = now;
    resetRateLimitGate();
    await dispose?.();
    h.restore();
  }
});
