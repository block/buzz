import assert from "node:assert/strict";
import test from "node:test";

import {
  countLanes,
  fibresInLane,
  moveFibreStatus,
  normalizeFibresResponse,
} from "@/features/triage/fibreStatus";

const openFibre = {
  id: "f1",
  kind: "ask",
  status: "open",
  score: 80,
  engagement: 0,
  lane: "important",
  title: "Open ask",
  summary: "",
  why: "",
  whyShort: "",
  signals: [],
  channelId: "c1",
  channelName: "c1",
  isDm: false,
  people: [],
  artifacts: [],
  createdAt: 1,
  updatedAt: 1,
};

function payload(overrides = {}) {
  return {
    fibres: [],
    done: [],
    openCount: 0,
    doneCount: 0,
    clearedCount: 0,
    laneCounts: { important: 0, hot: 0, other: 0 },
    ingested: 0,
    ...overrides,
  };
}

test("moveFibreStatus moves an open fibre into done", () => {
  const next = moveFibreStatus(
    payload({
      fibres: [openFibre],
      openCount: 1,
    }),
    "f1",
    "done",
  );
  assert.equal(next.fibres.length, 0);
  assert.equal(next.done.length, 1);
  assert.equal(next.done[0].status, "done");
  assert.equal(next.openCount, 0);
  assert.equal(next.doneCount, 1);
  assert.equal(next.clearedCount, 1);
});

test("normalizeFibresResponse keeps cached done when the engine omits it", () => {
  const previous = payload({
    done: [{ ...openFibre, status: "done" }],
    doneCount: 1,
    clearedCount: 1,
  });
  const next = normalizeFibresResponse(
    { fibres: [], openCount: 0, clearedCount: 1 },
    previous,
  );
  assert.equal(next.done.length, 1);
  assert.equal(next.done[0].id, "f1");
  assert.equal(next.doneCount, 1);
});

test("normalizeFibresResponse drops cached done items that are open again", () => {
  const previous = payload({
    done: [{ ...openFibre, status: "done" }],
    doneCount: 1,
    clearedCount: 1,
  });
  const next = normalizeFibresResponse(
    { fibres: [openFibre], openCount: 1, clearedCount: 0 },
    previous,
  );
  assert.equal(next.done.length, 0);
  assert.equal(next.fibres[0].id, "f1");
});

test("normalizeFibresResponse trusts an explicit empty done list", () => {
  const previous = payload({
    done: [{ ...openFibre, status: "done" }],
    doneCount: 1,
  });
  const next = normalizeFibresResponse(
    { fibres: [], done: [], openCount: 0, doneCount: 0 },
    previous,
  );
  assert.equal(next.done.length, 0);
});

test("normalizeFibresResponse injects a patched done fibre when done is omitted", () => {
  const next = normalizeFibresResponse({
    fibres: [],
    fibre: { ...openFibre, status: "done" },
    openCount: 0,
    clearedCount: 1,
  });
  assert.equal(next.done[0].id, "f1");
  assert.equal(next.done[0].status, "done");
});

test("lane counts follow a fibre out of the open list", () => {
  const hot = { ...openFibre, id: "f2", lane: "hot" };
  const next = moveFibreStatus(
    payload({
      fibres: [openFibre, hot],
      openCount: 2,
      laneCounts: { important: 1, hot: 1, other: 0 },
    }),
    "f1",
    "done",
  );

  assert.deepEqual(next.laneCounts, { important: 0, hot: 1, other: 0 });
});

test("a fibre from an engine without lanes falls back to other", () => {
  const legacy = { ...openFibre, lane: undefined };
  assert.deepEqual(countLanes([legacy]), {
    important: 0,
    hot: 0,
    other: 1,
  });
  assert.deepEqual(fibresInLane([legacy], "other"), [legacy]);
  assert.deepEqual(fibresInLane([legacy], "important"), []);
});

test("normalizeFibresResponse derives lane counts the engine omitted", () => {
  const next = normalizeFibresResponse({
    fibres: [openFibre, { ...openFibre, id: "f2", lane: "other" }],
    openCount: 2,
  });

  assert.deepEqual(next.laneCounts, { important: 1, hot: 0, other: 1 });
});

test("moveFibreStatus reopens a done fibre", () => {
  const next = moveFibreStatus(
    payload({
      done: [{ ...openFibre, status: "done" }],
      doneCount: 1,
      clearedCount: 1,
    }),
    "f1",
    "open",
  );
  assert.equal(next.done.length, 0);
  assert.equal(next.fibres[0].status, "open");
  assert.equal(next.openCount, 1);
  assert.equal(next.clearedCount, 0);
});
