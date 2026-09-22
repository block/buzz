import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
const frames = new Map();
let frameId = 0;
before(() =>
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
    requestAnimationFrame: (callback) => {
      frames.set(++frameId, callback);
      return frameId;
    },
    cancelAnimationFrame: (id) => frames.delete(id),
  }),
);
afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  frames.clear();
});
after(() => dom.window.close());

for (const action of [
  "unchanged",
  "authored edit then clear",
  "unpin",
  "turn off",
  "switch thread",
  "switch channel",
  "unpin then repin",
  "turn off then on",
  "switch thread then back",
]) {
  test(`successful-send frame respects ${action}`, async () => {
    const { act, renderHook } = await import("@testing-library/react");
    const { useAddressedAgentMentionRestore } = await import(
      "./useAddressedAgentMentionRestore.ts"
    );
    let revision = 0;
    const calls = [];
    const initialProps = {
      audiencePubkeys: ["agent-a"],
      channelId: "channel-a",
      enabled: true,
      audienceScope: "thread-a",
      getComposerRevision: () => revision,
    };
    const { result, rerender } = renderHook(useAddressedAgentMentionRestore, {
      initialProps,
    });
    result.current.restoreAddressedAgentMentionsRef.current = (...args) => {
      calls.push(args);
      return "@Agent ";
    };
    act(() =>
      result.current.onAddressedAgentsSendSucceeded(["agent-a"], ["agent-a"]),
    );
    assert.equal(
      frames.size,
      1,
      "real hook schedules successful-send restoration",
    );
    if (action === "authored edit then clear") revision += 2;
    if (action === "unpin") rerender({ ...initialProps, audiencePubkeys: [] });
    if (action === "turn off") rerender({ ...initialProps, enabled: false });
    if (action === "switch thread")
      rerender({ ...initialProps, audienceScope: "thread-b" });
    if (action === "switch channel")
      rerender({ ...initialProps, channelId: "channel-b" });
    if (action === "unpin then repin") {
      rerender({ ...initialProps, audiencePubkeys: [] });
      rerender(initialProps);
    }
    if (action === "turn off then on") {
      rerender({ ...initialProps, enabled: false });
      rerender(initialProps);
    }
    if (action === "switch thread then back") {
      rerender({ ...initialProps, audienceScope: "thread-b" });
      rerender(initialProps);
    }
    act(() => {
      const callbacks = [...frames.values()];
      frames.clear();
      for (const callback of callbacks) callback();
    });
    assert.deepEqual(
      calls,
      action === "unchanged" ? [[["agent-a"], ["agent-a"]]] : [],
    );
  });
}
