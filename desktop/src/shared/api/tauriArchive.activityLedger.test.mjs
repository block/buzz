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
      totalObserverFrames: 1,
      hasMore: false,
      nextBeforeCreatedAt: 2,
      nextBeforeId: "a".repeat(64),
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

  assert.equal(calls, 1);
  assert.deepEqual(
    pages.map((page) => page.unindexedObserverFrames),
    [3],
  );
  assert.equal(
    pages.reduce((total, page) => total + page.unindexedObserverFrames, 0),
    3,
  );
  assert.equal(pages.at(-1).events.length, 1);
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
      totalObserverFrames: 3,
      hasMore: true,
      nextBeforeCreatedAt: 2,
      nextBeforeId: "a".repeat(64),
    },
    {
      events: [event("d", 4), event("b", 3)],
      backfillComplete: true,
      unindexedObserverFrames: 0,
      archiveRevision: 2,
      restartRequired: true,
      totalObserverFrames: 3,
      hasMore: true,
      nextBeforeCreatedAt: 3,
      nextBeforeId: "b".repeat(64),
    },
    {
      events: [event("b", 3)],
      backfillComplete: true,
      unindexedObserverFrames: 0,
      archiveRevision: 2,
      restartRequired: false,
      totalObserverFrames: 1,
      hasMore: false,
      nextBeforeCreatedAt: 3,
      nextBeforeId: "b".repeat(64),
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

test("observer range iterator counts malformed archived JSON rows", async (t) => {
  const previousWindow = globalThis.window;
  t.after(() => {
    globalThis.window = previousWindow;
  });
  globalThis.window ??= {};
  window.__TAURI_INTERNALS__ = {
    invoke() {
      return Promise.resolve({
        events: ["{invalid-json"],
        backfillComplete: true,
        unindexedObserverFrames: 0,
        archiveRevision: 9,
        restartRequired: false,
        totalObserverFrames: 1,
        hasMore: false,
        nextBeforeCreatedAt: 2,
        nextBeforeId: "f".repeat(64),
      });
    },
  };

  const pages = [];
  for await (const page of iterateArchivedObserverEventPagesForRange({
    startCreatedAt: 1,
    endCreatedAt: 3,
  })) {
    pages.push(page);
  }
  assert.equal(pages.length, 1);
  assert.equal(pages[0].events.length, 0);
  assert.equal(pages[0].rejectedArchiveRows, 1);
});

test("observer range iterator falls back to a bounded newest page under sustained ingest", async (t) => {
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
  const page = (revision, restartRequired) => ({
    events: [event(String(revision % 10), revision), event("a", 1)],
    backfillComplete: true,
    unindexedObserverFrames: 0,
    archiveRevision: revision,
    restartRequired,
    totalObserverFrames: 10,
    hasMore: true,
    nextBeforeCreatedAt: 1,
    nextBeforeId: "a".repeat(64),
  });
  const responses = [
    page(1, false),
    page(2, true),
    page(3, true),
    page(4, true),
    page(5, true),
  ];
  let calls = 0;
  globalThis.window ??= {};
  window.__TAURI_INTERNALS__ = {
    invoke() {
      const response = responses[calls];
      calls += 1;
      return Promise.resolve(response);
    },
  };

  const pages = [];
  for await (const item of iterateArchivedObserverEventPagesForRange({
    startCreatedAt: 1,
    endCreatedAt: 20,
    pageSize: 2,
  })) {
    pages.push(item);
  }
  assert.equal(calls, 5);
  assert.equal(pages.filter((item) => item.reset).length, 4);
  assert.equal(pages.at(-1).archiveRevision, 5);
  assert.equal(pages.at(-1).events.length, 2);
  assert.equal(pages.at(-1).omittedObserverFrames, 8);
});
