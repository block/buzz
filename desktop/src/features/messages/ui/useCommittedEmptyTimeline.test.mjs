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

async function renderTimelineState(initialProps) {
  const { renderHook } = await import("@testing-library/react");
  const { useCommittedEmptyTimeline } = await import(
    "./useCommittedEmptyTimeline.ts"
  );
  return renderHook((props) => useCommittedEmptyTimeline(props), {
    initialProps,
  });
}

const empty = {
  channelId: "channel-a",
  deferredCount: 0,
  hasPersistentIntro: true,
  isLoading: false,
  liveCount: 0,
};

test("only preserves an intro after an empty timeline commits", async () => {
  const { result, rerender } = await renderTimelineState(empty);

  assert.equal(result.current, false);
  rerender({ ...empty, liveCount: 1 });
  assert.equal(result.current, true);
  rerender({ ...empty, deferredCount: 1, liveCount: 1 });
  assert.equal(result.current, false);
  rerender({ ...empty, liveCount: 1 });
  assert.equal(result.current, false);
});

test("a committed empty proof never carries across channels", async () => {
  const { result, rerender } = await renderTimelineState(empty);

  rerender({ ...empty, channelId: "channel-b", liveCount: 1 });
  assert.equal(result.current, false);
});

test("loading and deferred-stale commits cannot establish empty proof", async () => {
  const { result, rerender } = await renderTimelineState({
    ...empty,
    isLoading: true,
  });

  rerender({ ...empty, liveCount: 1 });
  assert.equal(result.current, false);
});
