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

const topLevelMessage = {
  content: "message",
  created_at: 1_724_457_600,
  id: "message-id",
  kind: 9,
  pubkey: "author",
  sig: "signature",
  tags: [["h", "general"]],
};

async function mountReadMarker(enabled) {
  const { renderHook } = await import("@testing-library/react");
  const { useHuddleReadMarker } = await import("./useHuddleReadMarker.ts");
  const calls = [];
  const view = renderHook(() =>
    useHuddleReadMarker({
      activeChannelId: "general",
      activeChannelIsMember: true,
      enabled,
      isHuddleTranscript: false,
      markChannelRead: (...args) => calls.push(args),
      messages: [topLevelMessage],
      resolvedMessages: [topLevelMessage],
    }),
  );
  return { calls, view };
}

test("disabled activity companions do not advance the huddle read marker", async () => {
  const { calls } = await mountReadMarker(false);
  assert.deepEqual(calls, []);
});

test("enabled channel screens retain the top-level read marker", async () => {
  const { calls } = await mountReadMarker(true);
  assert.deepEqual(calls, [
    ["general", "2024-08-24T00:00:00.000Z", { topLevelOnly: true }],
  ]);
});
