import assert from "node:assert/strict";
import test from "node:test";

import {
  __resetBestieJobStoreForTests,
  addBestieJobForScope,
  applyBestieJobActionsFromAgentMessage,
  applyBestieJobIntentFromUserMessage,
  getBestieJobState,
} from "./bestieJobStore.ts";

const SCOPE = {
  agentPubkey: "c".repeat(64),
  ownerPubkey: "d".repeat(64),
  relayUrl: "wss://relay.test",
};

function mockStorage() {
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
}

test("casual NL schedule does not auto-create a job", () => {
  mockStorage();
  __resetBestieJobStoreForTests();
  const nowMs = Date.parse("2026-09-27T12:00:00.000-07:00");
  const nl = applyBestieJobIntentFromUserMessage(
    SCOPE,
    "u1",
    "Schedule a job in 5 minutes to summarize my inbox",
    nowMs,
  );
  assert.equal(nl, 0);
  assert.equal(getBestieJobState(SCOPE).jobs.length, 0);
});

test("unconfirmed fence does not create; confirmed fence creates", () => {
  mockStorage();
  __resetBestieJobStoreForTests();
  const dueAt = Math.floor(Date.parse("2026-09-27T12:00:00.000-07:00") / 1000) + 300;
  const draft = `\`\`\`bestie-job
{"op":"add","job":{"title":"summarize my inbox","prompt":"summarize my inbox","schedule":{"kind":"once","dueAt":${dueAt}}}}
\`\`\``;
  assert.equal(applyBestieJobActionsFromAgentMessage(SCOPE, "a0", draft), 0);
  assert.equal(getBestieJobState(SCOPE).jobs.length, 0);

  const confirmed = `\`\`\`bestie-job
{"op":"add","confirmed":true,"job":{"title":"summarize my inbox","prompt":"summarize my inbox","schedule":{"kind":"once","dueAt":${dueAt}}}}
\`\`\``;
  assert.equal(applyBestieJobActionsFromAgentMessage(SCOPE, "a1", confirmed), 1);
  assert.equal(getBestieJobState(SCOPE).jobs.length, 1);
});

test("RHS/manual add still creates form-complete jobs directly", () => {
  mockStorage();
  __resetBestieJobStoreForTests();
  addBestieJobForScope(SCOPE, {
    prompt: "Ping channel C",
    schedule: { everySeconds: 3600, kind: "interval" },
    title: "Ping",
  });
  assert.equal(getBestieJobState(SCOPE).jobs.length, 1);
});

test("NL cancel still removes a matching job", () => {
  mockStorage();
  __resetBestieJobStoreForTests();
  addBestieJobForScope(SCOPE, {
    prompt: "x",
    schedule: { everySeconds: 60, kind: "interval" },
    title: "Inbox",
  });
  const removed = applyBestieJobIntentFromUserMessage(
    SCOPE,
    "u2",
    'Cancel job "Inbox"',
  );
  assert.equal(removed, 1);
  assert.equal(getBestieJobState(SCOPE).jobs.length, 0);
});
