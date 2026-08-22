import assert from "node:assert/strict";
import test from "node:test";

import { iterateArchivedObserverEventPagesForRange } from "./tauriArchive.ts";

test("observer range iterator fences and accumulates concurrent exclusions", async (t) => {
  const previousWindow = globalThis.window;
  t.after(() => {
    globalThis.window = previousWindow;
  });

  const rawEvent = JSON.stringify({
    id: "a".repeat(64),
    pubkey: "agent-a",
    created_at: 2,
    kind: 24200,
    tags: [],
    content: "encrypted",
    sig: "signature",
  });
  const responses = [
    {
      events: [rawEvent],
      backfillComplete: true,
      unindexedObserverFrames: 1,
    },
    {
      events: [],
      backfillComplete: true,
      unindexedObserverFrames: 3,
    },
    {
      events: [],
      backfillComplete: true,
      unindexedObserverFrames: 4,
    },
  ];
  let calls = 0;
  globalThis.window ??= {};
  window.__TAURI_INTERNALS__ = {
    invoke(command) {
      assert.equal(command, "read_archived_observer_events_for_range");
      const response = responses[calls];
      calls += 1;
      return Promise.resolve(response);
    },
  };

  const pages = [];
  for await (const page of iterateArchivedObserverEventPagesForRange({
    startCreatedAt: 1,
    endCreatedAt: 3,
    pageSize: 1,
  })) {
    pages.push(page);
  }

  assert.equal(calls, 3, "last event page must be followed by a count fence");
  assert.deepEqual(
    pages.map((page) => page.unindexedObserverFrames),
    [1, 2, 1],
  );
  assert.equal(
    pages.reduce((total, page) => total + page.unindexedObserverFrames, 0),
    4,
  );
  assert.equal(pages.at(-1).events.length, 0);
});
