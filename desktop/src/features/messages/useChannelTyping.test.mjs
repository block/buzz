import assert from "node:assert/strict";
import { after, afterEach, before, beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
let act, renderHook, cleanup, useChannelTyping, relayClient;
let callbacks, disposed, now, prune;
const originalNow = Date.now;
const bot = "b".repeat(64);
const self = "a".repeat(64);
const channel = { id: "forum-one", channelType: "forum" };
const event = (overrides = {}) => ({
  id: "event",
  pubkey: bot,
  kind: 20002,
  content: "",
  sig: "",
  created_at: Math.floor(now / 1000),
  tags: [
    ["h", "forum-one"],
    ["e", "post-one", "", "root"],
    ["e", "comment-one", "", "reply"],
  ],
  ...overrides,
});
before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  ({ act, renderHook, cleanup } = await import("@testing-library/react"));
  ({ useChannelTyping } = await import("./useChannelTyping.ts"));
  ({ relayClient } = await import("../../shared/api/relayClient.ts"));
});
beforeEach(() => {
  callbacks = new Map();
  disposed = [];
  now = 2_000_000_000_000;
  Date.now = () => now;
  relayClient.subscribeToTypingIndicators = async (id, callback) => {
    callbacks.set(id, callback);
    return async () => {
      disposed.push(id);
      callbacks.delete(id);
    };
  };
  dom.window.setInterval = (fn) => {
    prune = fn;
    return 1;
  };
  dom.window.clearInterval = () => {};
});
afterEach(() => {
  cleanup();
  Date.now = originalNow;
});
after(() => dom.window.close());
async function mount(type = "forum") {
  const view = renderHook(
    ({ selected, latest }) => useChannelTyping(selected, self, latest),
    {
      initialProps: {
        selected: { ...channel, channelType: type },
        latest: null,
      },
    },
  );
  await act(async () => {});
  return view;
}
test("Forums subscribe and map nested-comment typing to the post", async () => {
  const { result } = await mount();
  assert.equal(callbacks.has(channel.id), true);
  act(() => callbacks.get(channel.id)(event()));
  assert.deepEqual(result.current, [{ pubkey: bot, threadHeadId: "post-one" }]);
});
test("stream typing retains immediate-parent scope", async () => {
  const { result } = await mount("stream");
  act(() => callbacks.get(channel.id)(event()));
  assert.equal(result.current[0].threadHeadId, "comment-one");
});
test("self, stale, and other-channel signals are ignored", async () => {
  const { result } = await mount();
  act(() => {
    callbacks.get(channel.id)(event({ pubkey: self }));
    callbacks.get(channel.id)(event({ created_at: now / 1000 - 9 }));
    callbacks.get(channel.id)(event({ tags: [["h", "other-forum"]] }));
  });
  assert.deepEqual(result.current, []);
});
test("keepalive extends TTL and a stopped heartbeat expires", async () => {
  const { result } = await mount();
  act(() => callbacks.get(channel.id)(event()));
  now += 6000;
  act(() => callbacks.get(channel.id)(event()));
  now += 6000;
  act(() => prune());
  assert.equal(result.current.length, 1);
  now += 3000;
  act(() => prune());
  assert.deepEqual(result.current, []);
});
test("Forum completion clears only its post and suppresses delayed old heartbeats", async () => {
  const { result, rerender } = await mount();
  act(() => {
    callbacks.get(channel.id)(event());
    callbacks.get(channel.id)(
      event({
        tags: [
          ["h", channel.id],
          ["e", "post-two", "", "reply"],
        ],
      }),
    );
  });
  const old = event();
  now += 1000;
  rerender({ selected: channel, latest: event({ kind: 45003 }) });
  assert.deepEqual(
    result.current.map((entry) => entry.threadHeadId),
    ["post-two"],
  );
  act(() => callbacks.get(channel.id)(old));
  assert.equal(result.current.length, 1);
});
test("switching Forum clears state and disposes the old subscription", async () => {
  const { result, rerender } = await mount();
  act(() => callbacks.get(channel.id)(event()));
  rerender({ selected: { ...channel, id: "forum-two" }, latest: null });
  await act(async () => {});
  assert.deepEqual(result.current, []);
  assert.deepEqual(disposed, [channel.id]);
  assert.equal(callbacks.has("forum-two"), true);
});
