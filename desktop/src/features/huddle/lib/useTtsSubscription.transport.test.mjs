// Real main-window TTS hook -> RelayClient -> fake Tauri IPC and clock.
// The companion's huddle channel is deliberately NOT the main visible channel.
import assert from "node:assert/strict";
import { after, afterEach, beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
  pretendToBeVisual: true,
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  localStorage: dom.window.localStorage,
  IS_REACT_ACT_ENVIRONMENT: true,
});
const originalNow = Date.now;
const START = 1_000_000;
let now = START;
Date.now = () => now;
const timers = new Map();
let timerId = 1;
window.setTimeout = (fn, ms, ...args) => {
  const id = timerId++;
  timers.set(id, { fn, args, at: now + ms });
  return id;
};
window.setInterval = (fn, ms) => {
  const id = timerId++;
  timers.set(id, { fn, args: [], at: now + ms, interval: ms });
  return id;
};
window.clearTimeout = window.clearInterval = (id) => timers.delete(id);
const writes = [];
const spoken = [];
const callbacks = new Map();
let callbackId = 1;
window.__TAURI_INTERNALS__ = {
  transformCallback(callback) {
    const id = callbackId++;
    callbacks.set(id, callback);
    return id;
  },
  unregisterCallback(id) {
    callbacks.delete(id);
  },
  async invoke(command, args) {
    if (command === "plugin:websocket|send") {
      writes.push({ frame: JSON.parse(args.message.data), at: now });
      return;
    }
    if (command === "plugin:websocket|disconnect") return;
    if (command === "plugin:event|listen") return args.handler;
    if (command === "plugin:event|unlisten") {
      callbacks.delete(args.eventId);
      return;
    }
    if (command === "get_huddle_agent_pubkeys") return ["agent"];
    if (command === "get_huddle_state") return { tts_enabled: true };
    if (command === "speak_agent_message") {
      spoken.push(args);
      return;
    }
    assert.fail(`unexpected IPC: ${command}`);
  },
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener: (_event, id) => callbacks.delete(id),
};
const { act, cleanup, renderHook } = await import("@testing-library/react");
const { relayClient } = await import("@/shared/api/relayClient");
const { resetRateLimitGate } = await import("@/shared/api/relayRateLimitGate");
const { buildHuddleTtsLiveFilter } = await import(
  "@/shared/api/relayChannelFilters"
);
const { useTtsSubscription } = await import("./useTtsSubscription.ts");
const flush = async () => {
  for (let i = 0; i < 30; i++) await Promise.resolve();
};
const deliver = (frame) =>
  relayClient.handleWsMessage(
    { type: "Text", data: JSON.stringify(frame) },
    relayClient.connectionGeneration,
  );
const requests = () => writes.filter(({ frame }) => frame[0] === "REQ");
const ttsRequests = () =>
  requests().filter(({ frame }) => frame[2]["#h"]?.[0] === "huddle");
async function advance(ms) {
  await act(async () => {
    const target = now + ms;
    let fired = 0;
    for (;;) {
      const next = [...timers]
        .filter(([, t]) => t.at <= target)
        .sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
      if (!next) break;
      assert.ok(++fired < 1000, "timer loop must make progress");
      const [id, t] = next;
      now = t.at;
      if (t.interval) t.at += t.interval;
      else timers.delete(id);
      t.fn(...t.args);
      await flush();
    }
    now = target;
    await flush();
  });
}
beforeEach(async () => {
  now = START;
  writes.length = 0;
  spoken.length = 0;
  resetRateLimitGate();
  relayClient.wsId = 7;
  relayClient.setVisibleChannelId("main-timeline");
  for (let i = 0; i < 296; i++) {
    void relayClient
      .subscribeLive(
        { kinds: [9, 5, 7], "#h": [`cold-${i}`], since: 123, limit: 1000 },
        () => {},
      )
      .catch(() => {});
  }
  await flush();
  await advance(1000);
});
afterEach(async () => {
  await act(async () => {
    cleanup();
    await flush();
  });
  relayClient.disconnect();
  resetRateLimitGate();
  await advance(50);
  assert.equal(
    timers.size,
    0,
    "release readiness, drain and membership timers",
  );
  assert.equal(callbacks.size, 0, "release native listeners");
});
after(() => {
  Date.now = originalNow;
  dom.window.close();
});
async function mountTts() {
  const self = { current: "human" };
  let hook;
  await act(async () => {
    hook = renderHook(() => useTtsSubscription("huddle", self));
    await flush();
  });
  return hook;
}

for (const cooldown of [false, true]) {
  test(`companion TTS outranks 296 cold subscriptions without changing main visibility (cooldown=${cooldown})`, async () => {
    if (cooldown)
      await deliver(["NOTICE", "rate-limited: quota exceeded; retry in 4s"]);
    const count = requests().length;
    await mountTts();
    const expectedFilter = buildHuddleTtsLiveFilter("huddle", 996);
    await advance(cooldown ? 3999 : 249);
    assert.equal(
      requests().length,
      count,
      "priority must not bypass pacing or cooldown",
    );
    await advance(1);
    assert.equal(
      ttsRequests().length,
      1,
      "active speech must take the next permitted slot",
    );
    const { frame, at } = ttsRequests()[0];
    assert.equal(at, START + (cooldown ? 5000 : 1250));
    assert.deepEqual(
      frame[2],
      expectedFilter,
      "retain bounded replay, not limit:0",
    );
    assert.equal(
      relayClient.visibleChannelId,
      "main-timeline",
      "do not borrow timeline visibility",
    );
    assert.ok(requests().length < 296, "cold backlog still exists");
    const event = {
      id: "reply",
      kind: 9,
      pubkey: "agent",
      created_at: 1001,
      tags: [["h", "huddle"]],
      content: "Hello from the huddle",
      sig: "",
    };
    await act(async () => {
      await deliver(["EVENT", frame[1], event]);
      await deliver(["EOSE", frame[1]]);
      await flush();
    });
    assert.deepEqual(
      spoken.map(({ text }) => text),
      [event.content],
    );
    await advance(250);
    assert.ok(
      requests().at(-1).frame[2]["#h"][0].startsWith("cold-"),
      "background drain resumes",
    );
    // A quota-refused active subscription keeps the same priority and filter.
    await deliver([
      "CLOSED",
      frame[1],
      "rate-limited: quota exceeded; retry in 4s",
    ]);
    const beforeRetry = requests().length;
    await advance(3999);
    assert.equal(requests().length, beforeRetry);
    await advance(1);
    assert.deepEqual(
      ttsRequests()[1]?.frame,
      frame,
      "active retry takes next permitted slot",
    );
    await act(async () => {
      await deliver(["EVENT", frame[1], event]);
      await deliver(["EOSE", frame[1]]);
      await flush();
    });
    assert.equal(
      spoken.length,
      1,
      "stored/live or retry overlap is not spoken twice",
    );
    for (let i = 1; i < requests().length; i++)
      assert.ok(requests()[i].at - requests()[i - 1].at >= 250);
  });
}
