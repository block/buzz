import assert from "node:assert/strict";
import test from "node:test";

import { iterateArchivedObserverEventPagesForRange } from "./tauriArchive.ts";

test("observer range iterator discloses stable archive exclusions", async (t) => {
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
      unindexedObserverFrames: 3,
      archiveRevision: 5,
      restartRequired: false,
    },
    {
      events: [],
      backfillComplete: true,
      unindexedObserverFrames: 3,
      archiveRevision: 5,
      restartRequired: false,
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

  assert.equal(calls, 2);
  assert.deepEqual(
    pages.map((page) => page.unindexedObserverFrames),
    [3, 0],
  );
  assert.equal(
    pages.reduce((total, page) => total + page.unindexedObserverFrames, 0),
    3,
  );
  assert.equal(pages.at(-1).events.length, 0);
});

test("observer range iterator restarts when the archive revision changes", async (t) => {
  const previousWindow = globalThis.window;
  t.after(() => {
    globalThis.window = previousWindow;
  });

  const event = (id, createdAt) =>
    JSON.stringify({
      id: id.repeat(64),
      pubkey: "agent-a",
      created_at: createdAt,
      kind: 24200,
      tags: [],
      content: "encrypted",
      sig: "signature",
    });
  const responses = [
    {
      events: [event("c", 3), event("a", 2)],
      backfillComplete: true,
      unindexedObserverFrames: 0,
      archiveRevision: 1,
      restartRequired: false,
    },
    {
      events: [],
      backfillComplete: true,
      unindexedObserverFrames: 0,
      archiveRevision: 2,
      restartRequired: true,
    },
    {
      events: [event("b", 3)],
      backfillComplete: true,
      unindexedObserverFrames: 0,
      archiveRevision: 2,
      restartRequired: false,
    },
  ];
  const inputs = [];
  globalThis.window ??= {};
  window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      assert.equal(command, "read_archived_observer_events_for_range");
      inputs.push(args.input);
      return Promise.resolve(responses[inputs.length - 1]);
    },
  };

  const pages = [];
  for await (const page of iterateArchivedObserverEventPagesForRange({
    startCreatedAt: 1,
    endCreatedAt: 4,
    pageSize: 2,
  })) {
    pages.push(page);
  }

  assert.deepEqual(
    pages.map((page) => ({
      reset: page.reset,
      revision: page.archiveRevision,
      ids: page.events.map((item) => item.id),
    })),
    [
      {
        reset: false,
        revision: 1,
        ids: ["c".repeat(64), "a".repeat(64)],
      },
      { reset: true, revision: 2, ids: [] },
      { reset: false, revision: 2, ids: ["b".repeat(64)] },
    ],
  );
  assert.equal(inputs[0].archiveRevision, null);
  assert.equal(inputs[1].archiveRevision, 1);
  assert.equal(inputs[2].archiveRevision, 2);
  assert.equal(inputs[2].beforeCreatedAt, null);
});
