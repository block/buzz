import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

test("pulse versions restart per addressed agent", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAddressMentionPulse } = await import(
    "./useAddressMentionPulse.ts"
  );
  const { result } = renderHook(() => useAddressMentionPulse());

  act(() => result.current.pulseMany(["AGENT-A", "agent-a", "agent-b"]));
  assert.deepEqual(result.current.pulseVersionByPubkey, {
    "agent-a": 1,
    "agent-b": 1,
  });

  act(() => result.current.pulseOne("agent-a"));
  assert.deepEqual(result.current.pulseVersionByPubkey, {
    "agent-a": 2,
    "agent-b": 1,
  });

  act(() => result.current.shakeMany(["AGENT-A", "agent-a", "agent-b"]));
  assert.deepEqual(result.current.shakeVersionByPubkey, {
    "agent-a": 1,
    "agent-b": 1,
  });
});
