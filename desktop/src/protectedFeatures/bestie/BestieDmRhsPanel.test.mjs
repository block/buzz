import assert from "node:assert/strict";
import test from "node:test";

import {
  bestieCategoryTitle,
  bestieIdleAuxiliaryKind,
  presentBestieContextCount,
  soonestEnabledJobDueAt,
  soonestOpenReminderDueAt,
} from "./bestieDmRhsHelpers.ts";

test("bestieCategoryTitle labels all RHS categories", () => {
  assert.equal(bestieCategoryTitle("reminder"), "Reminders");
  assert.equal(bestieCategoryTitle("todo"), "To-dos");
  assert.equal(bestieCategoryTitle("job"), "Jobs");
  assert.equal(bestieCategoryTitle("coffee"), "Coffee");
  assert.equal(bestieCategoryTitle("thread"), "Threads");
  assert.equal(bestieCategoryTitle("scratch"), "Scratch");
});

test("presentBestieContextCount omits empty like Projects overview", () => {
  assert.equal(presentBestieContextCount(undefined), undefined);
  assert.equal(presentBestieContextCount(0), undefined);
  assert.equal(presentBestieContextCount(3), 3);
});

test("bestieIdleAuxiliaryKind opens slide only for a drilled category", () => {
  assert.equal(bestieIdleAuxiliaryKind(null), null);
  assert.equal(bestieIdleAuxiliaryKind("thread"), "thread");
});

test("soonestOpenReminderDueAt picks earliest open reminder due", () => {
  const now = 1_700_000_000;
  const state = {
    items: [
      {
        createdAt: now,
        dayKey: null,
        dueAt: now + 600,
        id: "r2",
        kind: "reminder",
        sortOrder: 0,
        sourceMessageId: null,
        starred: false,
        status: "open",
        text: "later",
        updatedAt: now,
      },
      {
        createdAt: now,
        dayKey: null,
        dueAt: now + 120,
        id: "r1",
        kind: "reminder",
        sortOrder: 0,
        sourceMessageId: null,
        starred: false,
        status: "open",
        text: "soon",
        updatedAt: now,
      },
      {
        createdAt: now,
        dayKey: null,
        dueAt: now + 30,
        id: "r-done",
        kind: "reminder",
        sortOrder: 0,
        sourceMessageId: null,
        starred: false,
        status: "done",
        text: "done",
        updatedAt: now,
      },
    ],
    processedMessageIds: [],
    version: 1,
  };
  assert.equal(soonestOpenReminderDueAt(state), now + 120);
});

test("soonestEnabledJobDueAt picks earliest enabled nextDueAt", () => {
  const now = 1_700_000_000;
  const state = {
    firedSlotIds: [],
    jobs: [
      {
        createdAt: now,
        enabled: true,
        id: "j2",
        lastRunAt: null,
        nextDueAt: now + 900,
        prompt: "p",
        schedule: { dueAt: now + 900, kind: "once" },
        sourceMessageId: null,
        title: "later",
        updatedAt: now,
      },
      {
        createdAt: now,
        enabled: true,
        id: "j1",
        lastRunAt: null,
        nextDueAt: now + 180,
        prompt: "p",
        schedule: { dueAt: now + 180, kind: "once" },
        sourceMessageId: null,
        title: "soon",
        updatedAt: now,
      },
      {
        createdAt: now,
        enabled: false,
        id: "j-off",
        lastRunAt: null,
        nextDueAt: now + 60,
        prompt: "p",
        schedule: { dueAt: now + 60, kind: "once" },
        sourceMessageId: null,
        title: "off",
        updatedAt: now,
      },
    ],
    processedMessageIds: [],
    version: 1,
  };
  assert.equal(soonestEnabledJobDueAt(state), now + 180);
});
