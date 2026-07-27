import assert from "node:assert/strict";
import test from "node:test";

import { submitSpawnerCredential } from "./spawnerCredentialSubmit.ts";

const SPAWNER = "a".repeat(64);

test("savesWhenTheSpawnerAcksAcceptance", async () => {
  const calls = [];
  const result = await submitSpawnerCredential(
    {
      send: async (input) => {
        calls.push(input);
      },
      waitForAck: async () => ({ accepted: true }),
    },
    SPAWNER,
    "sk-ant-oat01-x",
    1_000,
  );
  assert.deepEqual(result, { kind: "saved", cleared: false });
  assert.deepEqual(calls, [
    { spawnerPubkey: SPAWNER, credential: "sk-ant-oat01-x" },
  ]);
});

test("anEmptyCredentialReportsCleared", async () => {
  const result = await submitSpawnerCredential(
    {
      send: async () => {},
      waitForAck: async () => ({ accepted: true }),
    },
    SPAWNER,
    "",
    1_000,
  );
  assert.deepEqual(result, { kind: "saved", cleared: true });
});

test("aRejectingAckSurfacesItsMessage", async () => {
  const result = await submitSpawnerCredential(
    {
      send: async () => {},
      waitForAck: async () => ({ accepted: false, message: "disk full" }),
    },
    SPAWNER,
    "sk-ant-api03-x",
    1_000,
  );
  assert.deepEqual(result, { kind: "error", message: "disk full" });
});

test("aTimedOutAckBecomesAnErrorResult", async () => {
  const result = await submitSpawnerCredential(
    {
      send: async () => {},
      waitForAck: () =>
        Promise.reject(
          new Error("The server did not confirm the credential in time."),
        ),
    },
    SPAWNER,
    "sk-ant-oat01-x",
    1_000,
  );
  assert.equal(result.kind, "error");
  assert.match(result.message, /did not confirm/);
});

test("aFailedSendDoesNotLeaveAnUnhandledAckRejection", async () => {
  const result = await submitSpawnerCredential(
    {
      send: () => Promise.reject(new Error("relay unreachable")),
      waitForAck: () => Promise.reject(new Error("timeout")),
    },
    SPAWNER,
    "sk-ant-oat01-x",
    1_000,
  );
  assert.equal(result.kind, "error");
  assert.match(result.message, /relay unreachable/);
});
