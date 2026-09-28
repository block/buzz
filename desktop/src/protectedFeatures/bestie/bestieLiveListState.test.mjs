import assert from "node:assert/strict";
import test from "node:test";

import {
  BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER,
  formatBestieLiveListStateHint,
  stripBestieOutboundHints,
  withBestieLiveListStateHint,
} from "./bestieLiveListState.ts";
import { BESTIE_LIST_TURN_HINT_MARKER } from "./bestieListProtocol.ts";
import { BESTIE_JOB_TURN_HINT_MARKER } from "./bestieJobProtocol.ts";

function todo(overrides = {}) {
  return {
    createdAt: 1,
    dayKey: "2026-09-27",
    dueAt: null,
    id: "todo-1",
    kind: "todo",
    repeat: null,
    sortOrder: 0,
    sourceMessageId: null,
    starred: false,
    status: "open",
    text: "Buy milk",
    updatedAt: 1,
    ...overrides,
  };
}

function reminder(overrides = {}) {
  return {
    createdAt: 1,
    dayKey: null,
    dueAt: 1_700_000_000,
    id: "rem-1",
    kind: "reminder",
    repeat: null,
    sortOrder: 0,
    sourceMessageId: null,
    starred: false,
    status: "open",
    text: "Stretch",
    updatedAt: 1,
    ...overrides,
  };
}

function job(overrides = {}) {
  return {
    createdAt: 1,
    enabled: true,
    id: "job-1",
    lastRunAt: null,
    nextDueAt: 1_700_000_100,
    prompt: "Summarize inbox",
    schedule: { hour: 9, kind: "daily", minute: 0 },
    sourceMessageId: null,
    title: "Morning brief",
    updatedAt: 1,
    ...overrides,
  };
}

test("formatBestieLiveListStateHint teaches live RHS authority and lists items", () => {
  const hint = formatBestieLiveListStateHint({
    jobs: [job(), job({ enabled: false, id: "job-2", title: "Off" })],
    openReminders: [reminder()],
    openTodos: [todo({ starred: true }), todo({ id: "todo-2", text: "Ship PR" })],
  });

  assert.match(hint, new RegExp(BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER.replace(/[[\]]/g, "\\$&")));
  assert.match(hint, /authoritative/i);
  assert.match(hint, /Do not claim you cannot query live state/i);
  assert.match(hint, /Open to-dos \(2\)/);
  assert.match(hint, /Buy milk/);
  assert.match(hint, /★/);
  assert.match(hint, /Ship PR/);
  assert.match(hint, /Open reminders \(1\)/);
  assert.match(hint, /Stretch/);
  assert.match(hint, /Jobs summary \(1 enabled \/ 2 total\)/);
  assert.match(hint, /Morning brief/);
  assert.doesNotMatch(hint, /\bOff\b/);
});

test("formatBestieLiveListStateHint renders empty sections", () => {
  const hint = formatBestieLiveListStateHint({
    jobs: [],
    openReminders: [],
    openTodos: [],
  });
  assert.match(hint, /Open to-dos \(0\):\n\(none\)/);
  assert.match(hint, /Open reminders \(0\):\n\(none\)/);
  assert.match(hint, /Jobs summary \(0 enabled \/ 0 total\):\n\(none enabled; 0 total\)/);
});

test("formatBestieLiveListStateHint caps long lists", () => {
  const openTodos = Array.from({ length: 5 }, (_, i) =>
    todo({ id: `t-${i}`, text: `Task ${i}` }),
  );
  const hint = formatBestieLiveListStateHint(
    { jobs: [], openReminders: [], openTodos },
    { maxItems: 2 },
  );
  assert.match(hint, /Task 0/);
  assert.match(hint, /Task 1/);
  assert.doesNotMatch(hint, /Task 2/);
  assert.match(hint, /…and 3 more/);
});

test("withBestieLiveListStateHint appends once; strip restores user text", () => {
  const scope = {
    agentPubkey: "a".repeat(64),
    ownerPubkey: "b".repeat(64),
    relayUrl: "wss://example.test",
  };
  // withBestieLiveListStateHint reads live storage; without items it still appends empty snapshot.
  const body = "what's on my todo list?";
  const once = withBestieLiveListStateHint(body, scope);
  assert.match(once, new RegExp(BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER.replace(/[[\]]/g, "\\$&")));
  assert.equal(withBestieLiveListStateHint(once, scope), once);
  assert.equal(stripBestieOutboundHints(once), body);
  assert.equal(withBestieLiveListStateHint(body, null), body);
});

test("stripBestieOutboundHints removes live + list + job markers from earliest", () => {
  const body = "Remind me to stretch in 5 minutes";
  const content = `${body}

${BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER}
Live state…

${BESTIE_LIST_TURN_HINT_MARKER}
List teach…

${BESTIE_JOB_TURN_HINT_MARKER}
Job teach…`;
  assert.equal(stripBestieOutboundHints(content), body);
});

test("stripBestieOutboundHints tolerates null/undefined body (View thread)", () => {
  assert.equal(stripBestieOutboundHints(undefined), "");
  assert.equal(stripBestieOutboundHints(null), "");
  assert.equal(stripBestieOutboundHints(""), "");
});

