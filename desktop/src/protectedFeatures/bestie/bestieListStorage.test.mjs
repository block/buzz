import assert from "node:assert/strict";
import test from "node:test";

import {
  addBestieListItem,
  bestieListStorageKey,
  coreListTextForDedupe,
  dismissBestieReminderItems,
  dueReminders,
  emptyBestieListState,
  nextBestieReminderDueAt,
  parseBestieListState,
  presentBestieReminderRepeat,
  readBestieListState,
  snoozeBestieListItems,
  updateBestieListItemStatus,
  writeBestieListState,
} from "./bestieListStorage.ts";

const SCOPE = {
  agentPubkey: "A".repeat(64),
  ownerPubkey: "B".repeat(64),
  relayUrl: "wss://Example.COM/relay",
};

test("storage key is scoped by relay and owner only (no agent)", () => {
  const key = bestieListStorageKey(SCOPE);
  assert.match(key, /^buzz-bestie-list\.v1:/);
  assert.match(key, /b{64}/);
  assert.doesNotMatch(key, /:a{64}$/);
  assert.equal(
    bestieListStorageKey({ ...SCOPE, agentPubkey: "c".repeat(64) }),
    key,
  );
});

test("parseBestieListState accepts valid payloads only", () => {
  assert.equal(parseBestieListState(null), null);
  assert.equal(parseBestieListState({ version: 2, items: [] }), null);
  const parsed = parseBestieListState({
    version: 1,
    items: [
      {
        id: "1",
        kind: "todo",
        text: " Ship ",
        status: "open",
        createdAt: 1,
        updatedAt: 1,
        dueAt: null,
        sourceMessageId: null,
      },
      { id: "bad" },
    ],
    processedMessageIds: ["m1", 2],
  });
  assert.equal(parsed.items.length, 1);
  assert.equal(parsed.items[0].text, "Ship");
  assert.deepEqual(parsed.processedMessageIds, ["m1"]);
});

test("read/write round-trip through localStorage", () => {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };

  let state = emptyBestieListState();
  state = addBestieListItem(state, { kind: "todo", text: "One" }, 10);
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Two", dueAt: 5 },
    11,
  );
  writeBestieListState(SCOPE, state);
  const loaded = readBestieListState(SCOPE);
  assert.equal(loaded.items.length, 2);
  assert.equal(dueReminders(loaded, 5).length, 1);
  assert.equal(dueReminders(loaded, 4).length, 0);
});

test("addBestieListItem dedupes same kind+text within due window", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Stretch", dueAt: 1000 },
    900,
  );
  const again = addBestieListItem(
    state,
    { kind: "reminder", text: "stretch", dueAt: 1050 },
    910,
  );
  assert.equal(again.items.length, 1);
  assert.equal(again.items[0].text, "Stretch");
});

test("addBestieListItem dedupes recent open item even if due differs", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Call mom", dueAt: 1000 },
    900,
  );
  const again = addBestieListItem(
    state,
    { kind: "reminder", text: "Call mom", dueAt: 5000 },
    950,
  );
  assert.equal(again.items.length, 1);
});

test("migrates legacy agent-scoped list keys into owner scope", () => {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      key: (index) => [...memory.keys()][index] ?? null,
      get length() {
        return memory.size;
      },
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };

  const legacyKey = `${bestieListStorageKey(SCOPE)}:${SCOPE.agentPubkey.toLowerCase()}`;
  let legacy = emptyBestieListState();
  legacy = addBestieListItem(legacy, { kind: "todo", text: "Persist me" }, 10);
  memory.set(legacyKey, JSON.stringify(legacy));

  const loaded = readBestieListState(SCOPE);
  assert.equal(loaded.items.length, 1);
  assert.equal(loaded.items[0].text, "Persist me");
  assert.equal(memory.has(legacyKey), false);
  assert.ok(memory.has(bestieListStorageKey(SCOPE)));

  // Different agent same owner still sees the data
  const otherAgent = readBestieListState({
    ...SCOPE,
    agentPubkey: "d".repeat(64),
  });
  assert.equal(otherAgent.items[0].text, "Persist me");
});

test("coreListTextForDedupe strips leading clock and at/by phrases", () => {
  assert.equal(
    coreListTextForDedupe("8:10 to finish the nightly reports"),
    "finish the nightly reports",
  );
  assert.equal(
    coreListTextForDedupe("Finish the nightly reports"),
    "finish the nightly reports",
  );
  assert.equal(
    coreListTextForDedupe("stretch in 5 minutes"),
    "stretch",
  );
  assert.equal(
    coreListTextForDedupe("Call mom at 3pm"),
    "call mom",
  );
});

test("screenshot: clean text+dueAt dedupes agent time-in-text without dueAt", () => {
  // NL: "Finish the nightly reports" @ 8:10; agent fence: "8:10 to finish…" no dueAt.
  const dueAt = 1_700_000_000;
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Finish the nightly reports", dueAt },
    dueAt - 60,
  );
  const again = addBestieListItem(
    state,
    {
      kind: "reminder",
      text: "8:10 to finish the nightly reports",
      dueAt: null,
    },
    dueAt - 30,
  );
  assert.equal(again.items.length, 1);
  assert.equal(again.items[0].text, "Finish the nightly reports");
  assert.equal(again.items[0].dueAt, dueAt);
});

test("dedupe prefers keeping/upgrading to the item with dueAt", () => {
  const dueAt = 1_700_000_100;
  let state = emptyBestieListState();
  // Agent fence first: time embedded in text, no dueAt.
  state = addBestieListItem(
    state,
    {
      kind: "reminder",
      text: "8:10 to finish the nightly reports",
      dueAt: null,
    },
    dueAt - 60,
  );
  // NL second: clean text + dueAt — merge onto one row.
  const merged = addBestieListItem(
    state,
    { kind: "reminder", text: "Finish the nightly reports", dueAt },
    dueAt - 30,
  );
  assert.equal(merged.items.length, 1);
  assert.equal(merged.items[0].dueAt, dueAt);
  assert.equal(merged.items[0].text, "Finish the nightly reports");
});

test("dueAt close + similar text dedupes slight rewords", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Finish the nightly reports", dueAt: 1000 },
    900,
  );
  const again = addBestieListItem(
    state,
    { kind: "reminder", text: "finish nightly reports", dueAt: 1050 },
    910,
  );
  assert.equal(again.items.length, 1);
});

test("presentBestieReminderRepeat labels one-off vs daily", () => {
  assert.equal(presentBestieReminderRepeat(null), "One-off");
  assert.equal(presentBestieReminderRepeat({ kind: "daily" }), "Daily");
});

test("snoozeBestieListItems pushes dueAt from now", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Stretch", dueAt: 1000 },
    900,
  );
  const id = state.items[0].id;
  const snoozed = snoozeBestieListItems(state, [id], 15 * 60, 2000);
  assert.equal(snoozed.items[0].dueAt, 2000 + 15 * 60);
});

test("dismissBestieReminderItems completes one-off and advances daily", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { kind: "reminder", text: "Once", dueAt: 1000 },
    900,
  );
  state = addBestieListItem(
    state,
    {
      kind: "reminder",
      text: "Every day",
      dueAt: 1000,
      repeat: { kind: "daily" },
    },
    901,
  );
  const [onceId, dailyId] = state.items.map((item) => item.id).reverse();
  // items are prepended — find by text
  const once = state.items.find((item) => item.text === "Once");
  const daily = state.items.find((item) => item.text === "Every day");
  const next = dismissBestieReminderItems(state, [once.id, daily.id], 1000);
  assert.equal(next.items.find((item) => item.id === once.id).status, "done");
  const advanced = next.items.find((item) => item.id === daily.id);
  assert.equal(advanced.status, "open");
  assert.equal(advanced.dueAt, nextBestieReminderDueAt(1000, { kind: "daily" }));
});

test("completing daily reminder advances instead of done", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    {
      kind: "reminder",
      text: "Stand up",
      dueAt: 1000,
      repeat: { kind: "daily" },
    },
    900,
  );
  const id = state.items[0].id;
  const next = updateBestieListItemStatus(state, id, "done", 1000);
  assert.equal(next.items[0].status, "open");
  assert.equal(
    next.items[0].dueAt,
    nextBestieReminderDueAt(1000, { kind: "daily" }),
  );
});
