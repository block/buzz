import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};

const journal = await import("./directAgentCreationJournal.ts");
const owner = "f".repeat(64);

test("a begun request fails closed instead of running twice", () => {
  journal.beginDirectAgentCreation(owner, "request-1", "Example Agent");

  assert.deepEqual(journal.getDirectAgentCreationResult(owner, "request-1"), {
    requestId: "request-1",
    status: "failed",
    displayName: "Example Agent",
    message:
      "A previous attempt did not record a terminal result. Inspect the agent roster before retrying with a new request ID.",
  });
});

test("a terminal result replaces the processing sentinel", () => {
  journal.beginDirectAgentCreation(owner, "request-2", "Example Agent");
  const result = {
    requestId: "request-2",
    status: "created",
    displayName: "Example Agent",
    agentPubkey: "a".repeat(64),
    message: "created",
  };
  journal.recordDirectAgentCreationResult(owner, result);

  assert.deepEqual(
    journal.getDirectAgentCreationResult(owner, "request-2"),
    result,
  );
});

test("request IDs do not cross owner identities", () => {
  const otherOwner = "e".repeat(64);
  journal.beginDirectAgentCreation(owner, "shared-request", "Owner A Agent");

  assert.equal(
    journal.getDirectAgentCreationResult(otherOwner, "shared-request"),
    null,
  );
});
