import assert from "node:assert/strict";
import test from "node:test";

const { deliverFeedNotificationBatch } = await import(
  "./use-feed-desktop-notifications.ts"
);

test("an enabled restart waits for permission repair before delivering the feed batch", async () => {
  let releaseRepair;
  const repairPermission = new Promise((resolve) => {
    releaseRepair = resolve;
  });
  const delivered = [];

  const delivery = deliverFeedNotificationBatch(
    [{ id: "restart-alert" }],
    async () => {
      await repairPermission;
      return true;
    },
    async (item) => {
      delivered.push(item.id);
    },
  );

  await Promise.resolve();
  assert.deepEqual(delivered, []);

  releaseRepair();
  await delivery;
  assert.deepEqual(delivered, ["restart-alert"]);
});

test("a permission repair that is not granted suppresses the feed batch", async () => {
  const delivered = [];

  await deliverFeedNotificationBatch(
    [{ id: "blocked-alert" }],
    async () => false,
    async (item) => {
      delivered.push(item.id);
    },
  );

  assert.deepEqual(delivered, []);
});
