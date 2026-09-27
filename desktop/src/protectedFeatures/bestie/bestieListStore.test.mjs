import assert from "node:assert/strict";
import test from "node:test";

import {
  __resetBestieListStoreForTests,
  applyBestieListActionsFromAgentMessage,
  applyBestieListIntentFromUserMessage,
  getBestieListSnapshot,
  getBestieListState,
} from "./bestieListStore.ts";

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
  assert.equal(item.text, "stretch");
  assert.equal(item.dueAt, Math.floor(nowMs / 1000) + 5 * 60);
});
