import assert from "node:assert/strict";
import test from "node:test";

import { createNativeTeamSnapshotAcknowledgement } from "./nativeTeamSnapshotAcknowledgement.ts";

test("native snapshot acknowledgement resumes only after its FIFO head is removed", async () => {
  let resolveAcknowledgement;
  const acknowledgement = new Promise((resolve) => {
    resolveAcknowledgement = resolve;
  });
  const resumed = [];
  const controller = createNativeTeamSnapshotAcknowledgement(
    async () => acknowledgement,
  );

  const pending = controller.acknowledge("first", () => resumed.push("drain"));
  assert.equal(controller.isAcknowledging(), true);
  assert.equal(controller.requestDrain(), false);
  assert.deepEqual(resumed, []);

  resolveAcknowledgement(true);
  assert.equal(await pending, true);
  assert.equal(controller.isAcknowledging(), false);
  assert.deepEqual(resumed, ["drain"]);
});

test("native snapshot acknowledgement retries the current FIFO head after a false response", async () => {
  const resumed = [];
  const controller = createNativeTeamSnapshotAcknowledgement(async () => false);

  assert.equal(
    await controller.acknowledge("first", () => resumed.push("drain")),
    false,
  );
  assert.deepEqual(resumed, ["drain"]);
});

test("native snapshot acknowledgement does not replay a head after IPC failure", async () => {
  const resumed = [];
  const controller = createNativeTeamSnapshotAcknowledgement(async () => {
    throw new Error("IPC unavailable");
  });

  assert.equal(
    await controller.acknowledge("first", () => resumed.push("drain")),
    false,
  );
  assert.deepEqual(resumed, []);
});
