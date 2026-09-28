import assert from "node:assert/strict";
import test from "node:test";

import { coreListTextForDedupe } from "./bestieListStorage.ts";
import {
  __resetBestieListStoreForTests,
  applyBestieListActionsFromAgentMessage,
  applyBestieListIntentFromUserMessage,
  getBestieListSnapshot,
  getBestieListState,
} from "./bestieListStore.ts";
import { withBestieLiveListStateHint } from "./bestieLiveListState.ts";

const SCOPE = {
  agentPubkey: "c".repeat(64),
  ownerPubkey: "d".repeat(64),
  relayUrl: "wss://relay.test",
};

test("applyBestieListActionsFromAgentMessage is idempotent per message", () => {
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
  __resetBestieListStoreForTests();

  const content = `\`\`\`bestie-list
{"op":"add","items":[{"kind":"todo","text":"From agent"}]}
\`\`\``;
  const first = applyBestieListActionsFromAgentMessage(SCOPE, "msg-1", content);
  const second = applyBestieListActionsFromAgentMessage(
    SCOPE,
    "msg-1",
    content,
  );
  assert.equal(first, 1);
  assert.equal(second, 0);
  assert.equal(getBestieListState(SCOPE).items.length, 1);
  assert.equal(getBestieListState(SCOPE).items[0].text, "From agent");
});

test("getBestieListSnapshot returns stable empty for null scope (useSyncExternalStore)", () => {
  // Regression: a fresh empty object every getSnapshot made React throw
  // "Maximum update depth exceeded" when BestieWakeController mounted with
  // listScope still null (assignment / identity not ready yet).
  const first = getBestieListSnapshot(null);
  const second = getBestieListSnapshot(null);
  assert.equal(first, second);
  assert.equal(first.items.length, 0);
});

test("applyBestieListIntentFromUserMessage adds reminders from NL once", () => {
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
  __resetBestieListStoreForTests();

  const nowMs = Date.parse("2026-09-26T15:00:00.000-07:00");
  const first = applyBestieListIntentFromUserMessage(
    SCOPE,
    "user-1",
    "Remind me to stretch in 5 minutes",
    nowMs,
  );
  const second = applyBestieListIntentFromUserMessage(
    SCOPE,
    "user-1",
    "Remind me to stretch in 5 minutes",
    nowMs,
  );
  assert.equal(first, 1);
  assert.equal(second, 0);
  const item = getBestieListState(SCOPE).items[0];
  assert.equal(item.kind, "reminder");
  assert.equal(item.text, "Stretch");
  assert.equal(item.dueAt, Math.floor(nowMs / 1000) + 5 * 60);
});

test("NL user add + agent fence with same text creates one reminder", () => {
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
  __resetBestieListStoreForTests();

  const nowMs = Date.parse("2026-09-26T15:00:00.000-07:00");
  const fromNl = applyBestieListIntentFromUserMessage(
    SCOPE,
    "user-dup",
    "Remind me to water plants in 10 minutes",
    nowMs,
  );
  assert.equal(fromNl, 1);
  const dueAt = Math.floor(nowMs / 1000) + 10 * 60;
  const fence = `\`\`\`bestie-list
{"op":"add","items":[{"kind":"reminder","text":"water plants","dueAt":${dueAt}}]}
\`\`\``;
  const fromAgent = applyBestieListActionsFromAgentMessage(
    SCOPE,
    "agent-dup",
    fence,
  );
  assert.equal(fromAgent, 0);
  assert.equal(getBestieListState(SCOPE).items.length, 1);
  assert.equal(getBestieListState(SCOPE).items[0].text, "Water plants");
});

test("NL + agent fence different wording still one reminder (screenshot)", () => {
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
  __resetBestieListStoreForTests();

  // Simulate NL already applied with clean text + dueAt.
  const nowMs = Date.parse("2026-09-27T20:00:00.000-07:00");
  const dueAt = Math.floor(Date.parse("2026-09-27T20:10:00.000-07:00") / 1000);
  const fromNl = applyBestieListIntentFromUserMessage(
    SCOPE,
    "user-nightly",
    "Remind me to finish the nightly reports at 8:10pm",
    nowMs,
  );
  assert.equal(fromNl, 1);
  const nlItem = getBestieListState(SCOPE).items[0];
  assert.equal(nlItem.kind, "reminder");
  assert.match(nlItem.text.toLowerCase(), /finish the nightly reports/);
  assert.ok(nlItem.dueAt);

  // Agent fence uses different text and omits dueAt — must not create a second row.
  const fence = `\`\`\`bestie-list
{"op":"add","items":[{"kind":"reminder","text":"8:10 to finish the nightly reports"}]}
\`\`\``;
  const fromAgent = applyBestieListActionsFromAgentMessage(
    SCOPE,
    "agent-nightly",
    fence,
    nowMs,
  );
  assert.equal(fromAgent, 0);
  assert.equal(getBestieListState(SCOPE).items.length, 1);
  const kept = getBestieListState(SCOPE).items[0];
  assert.ok(kept.dueAt);
  assert.equal(coreListTextForDedupe(kept.text), "finish the nightly reports");
});

test("bare clock NL does not create; agent fence after confirm does", () => {
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
  __resetBestieListStoreForTests();

  const nowMs = Date.parse("2026-09-27T20:00:00.000-07:00");
  const fromNl = applyBestieListIntentFromUserMessage(
    SCOPE,
    "user-bare",
    "Remind me to run the nightly report at 8:36",
    nowMs,
  );
  assert.equal(fromNl, 0);
  assert.equal(getBestieListState(SCOPE).items.length, 0);

  const dueAt = Math.floor(Date.parse("2026-09-28T08:36:00.000-07:00") / 1000);
  const fence = `\`\`\`bestie-list
{"op":"add","items":[{"kind":"reminder","text":"Run the nightly report","dueAt":${dueAt}}]}
\`\`\``;
  const fromAgent = applyBestieListActionsFromAgentMessage(
    SCOPE,
    "agent-bare",
    fence,
  );
  assert.equal(fromAgent, 1);
  const item = getBestieListState(SCOPE).items[0];
  assert.equal(item.text, "Run the nightly report");
  assert.equal(item.dueAt, dueAt);
});

test("bare-clock pending + PM reply creates reminder with evening dueAt", () => {
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
  __resetBestieListStoreForTests();

  const evening = Date.parse("2026-09-27T20:40:00.000-07:00");
  const ask = applyBestieListIntentFromUserMessage(
    SCOPE,
    "user-bare",
    "Remind me to run the nightly report at 8:45",
    evening,
  );
  assert.equal(ask, 0);
  assert.equal(getBestieListState(SCOPE).items.length, 0);
  assert.ok(getBestieListState(SCOPE).pendingReminderConfirm);
  assert.equal(
    getBestieListState(SCOPE).pendingReminderConfirm.text,
    "Run the nightly report",
  );

  const confirm = applyBestieListIntentFromUserMessage(
    SCOPE,
    "user-pm",
    "PM",
    evening,
  );
  assert.equal(confirm, 1);
  const state = getBestieListState(SCOPE);
  assert.equal(state.pendingReminderConfirm, null);
  assert.equal(state.items.length, 1);
  const item = state.items[0];
  assert.equal(item.text, "Run the nightly report");
  const due = new Date(item.dueAt * 1000);
  assert.equal(due.getHours(), 20);
  assert.equal(due.getMinutes(), 45);
});

test("agent fence with PM prose + AM dueAt is reconciled to PM", () => {
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
  __resetBestieListStoreForTests();

  const evening = Date.parse("2026-09-27T20:40:00.000-07:00");
  applyBestieListIntentFromUserMessage(
    SCOPE,
    "user-bare-2",
    "Remind me to run the nightly report at 8:45",
    evening,
  );
  const wrongAm = Math.floor(
    Date.parse("2026-09-28T08:45:00.000-07:00") / 1000,
  );
  const fence = `Got it — 8:45 PM.

\`\`\`bestie-list
{"op":"add","items":[{"kind":"reminder","text":"Run the nightly report","dueAt":${wrongAm}}]}
\`\`\``;
  const applied = applyBestieListActionsFromAgentMessage(
    SCOPE,
    "agent-wrong-am",
    fence,
    evening,
  );
  assert.equal(applied, 1);
  const item = getBestieListState(SCOPE).items[0];
  const due = new Date(item.dueAt * 1000);
  assert.equal(due.getHours(), 20);
  assert.equal(due.getMinutes(), 45);
  assert.notEqual(item.dueAt, wrongAm);
  assert.equal(getBestieListState(SCOPE).pendingReminderConfirm, null);
});

test("live list turn hint does not block NL reminder apply", () => {
  __resetBestieListStoreForTests();
  const scope = {
    agentPubkey: "e".repeat(64),
    ownerPubkey: "f".repeat(64),
    relayUrl: "wss://relay.test/live-hint",
  };
  const userText = "Remind me to stretch in 5 minutes";
  const withHint = withBestieLiveListStateHint(userText, scope);
  assert.match(withHint, /Bestie live lists/);
  const applied = applyBestieListIntentFromUserMessage(
    scope,
    "user-live-hint-1",
    withHint,
  );
  assert.equal(applied, 1);
  const items = getBestieListState(scope).items.filter((item) => item.status === "open");
  assert.equal(items.length, 1);
  assert.match(items[0].text, /stretch/i);
});



test("dedupe prefers near client NL dueAt over far fence epoch", () => {
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
  __resetBestieListStoreForTests();
  const nowMs = Date.parse("2026-09-28T09:00:00.000-07:00");
  const nearDue = Math.floor(nowMs / 1000) + 5 * 60;
  const farDue = nearDue + 30 * 24 * 60 * 60;
  // Fence lands first with a far absolute epoch.
  applyBestieListActionsFromAgentMessage(
    SCOPE,
    "fence-far",
    "```bestie-list\n" +
      JSON.stringify({
        op: "add",
        items: [{ kind: "reminder", text: "Stretch", dueAt: farDue }],
      }) +
      "\n```",
    nowMs,
  );
  // Client NL "in 5 minutes" should win the near dueAt on dedupe merge.
  applyBestieListIntentFromUserMessage(
    SCOPE,
    "nl-near",
    "Remind me to stretch in 5 minutes",
    nowMs,
  );
  const items = getBestieListState(SCOPE).items.filter(
    (item) => item.kind === "reminder" && item.status === "open",
  );
  assert.equal(items.length, 1);
  assert.equal(items[0].dueAt, nearDue);
});
