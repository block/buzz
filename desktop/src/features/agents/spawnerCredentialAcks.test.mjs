import assert from "node:assert/strict";
import test from "node:test";

import {
  deliverSpawnerCredentialAck,
  resetSpawnerCredentialAcks,
  waitForSpawnerCredentialAck,
} from "./spawnerCredentialAcks.ts";

const SPAWNER = "a".repeat(64);

test.afterEach(() => {
  resetSpawnerCredentialAcks();
});

test("resolvesAWaiterWithTheNextAckFromThatSpawner", async () => {
  const waiting = waitForSpawnerCredentialAck(SPAWNER, 1_000);
  deliverSpawnerCredentialAck(SPAWNER, { accepted: true });
  assert.deepEqual(await waiting, { accepted: true });
});

test("ignoresAcksFromADifferentSpawner", async () => {
  const waiting = waitForSpawnerCredentialAck(SPAWNER, 30);
  deliverSpawnerCredentialAck("b".repeat(64), { accepted: true });
  await assert.rejects(waiting, /did not confirm/);
});

test("rejectsOnTimeout", async () => {
  const waiting = waitForSpawnerCredentialAck(SPAWNER, 20);
  await assert.rejects(waiting, /did not confirm/);
});

test("acksQueueInOrderForConsecutiveWaiters", async () => {
  const first = waitForSpawnerCredentialAck(SPAWNER, 1_000);
  const second = waitForSpawnerCredentialAck(SPAWNER, 1_000);
  deliverSpawnerCredentialAck(SPAWNER, { accepted: true });
  deliverSpawnerCredentialAck(SPAWNER, { accepted: false, message: "nope" });
  assert.deepEqual(await first, { accepted: true });
  assert.deepEqual(await second, { accepted: false, message: "nope" });
});

test("resetDropsPendingWaitersByRejectingThem", async () => {
  const waiting = waitForSpawnerCredentialAck(SPAWNER, 10_000);
  resetSpawnerCredentialAcks();
  await assert.rejects(waiting, /Community changed/);
});
