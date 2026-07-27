import assert from "node:assert/strict";
import test from "node:test";

import { classifyTurns, scopeItemsToThread } from "./threadTurnScope.ts";

const isInjected = (id) => id.startsWith("reply:") || id.startsWith("prompt:");
const mine = new Set(["m1", "m2"]);

const userRow = (turnId, messageId) => ({
  id: `user:c:${messageId}`,
  type: "message",
  role: "user",
  messageId,
  turnId,
});
const toolRow = (turnId, n) => ({ id: `tool:${n}`, type: "tool", turnId });

test("classifies own, foreign and unattributed turns", () => {
  const items = [
    userRow("t1", "m1"),
    toolRow("t1", 1),
    userRow("t2", "other"),
    toolRow("t2", 2),
    toolRow("t3", 3), // live turn: no user row yet
  ];
  const map = classifyTurns(items, mine);
  assert.equal(map.get("t1"), "own");
  assert.equal(map.get("t2"), "foreign");
  assert.equal(map.get("t3"), "unattributed");
});

test("keeps the live unattributed turn — the darkness regression", () => {
  const items = [userRow("t1", "m1"), toolRow("t1", 1), toolRow("live", 9)];
  const kept = scopeItemsToThread(items, mine, isInjected).map((i) => i.id);
  assert.equal(kept.includes("tool:9"), true);
});

test("drops another thread's turn", () => {
  const items = [userRow("t1", "m1"), userRow("t2", "other"), toolRow("t2", 2)];
  const kept = scopeItemsToThread(items, mine, isInjected).map((i) => i.id);
  assert.deepEqual(kept, ["user:c:m1"]);
});

test("keeps injected rows regardless of turn", () => {
  const items = [
    { id: "prompt:m1", type: "message", role: "user", messageId: "m1" },
    { id: "reply:agent1", type: "message", role: "assistant" },
    userRow("t2", "other"),
  ];
  const kept = scopeItemsToThread(items, mine, isInjected).map((i) => i.id);
  assert.deepEqual(kept, ["prompt:m1", "reply:agent1"]);
});

test("keeps session-level items that have no turnId", () => {
  const items = [{ id: "session:new", type: "lifecycle", turnId: null }];
  assert.equal(scopeItemsToThread(items, mine, isInjected).length, 1);
});

test("no scoping when the thread id set is empty or absent", () => {
  const items = [userRow("t2", "other")];
  assert.equal(scopeItemsToThread(items, new Set(), isInjected).length, 1);
  assert.equal(scopeItemsToThread(items, undefined, isInjected).length, 1);
});

test("a turn with both own and foreign user rows counts as own", () => {
  const items = [userRow("t1", "m1"), userRow("t1", "other"), toolRow("t1", 1)];
  const kept = scopeItemsToThread(items, mine, isInjected).map((i) => i.id);
  assert.equal(kept.includes("tool:1"), true);
});
