import assert from "node:assert/strict";
import test from "node:test";

const { deliverFeedNotificationBatch, ensureFeedNotificationPermission } =
  await import("./use-feed-desktop-notifications.ts");

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

test("a concurrent feed batch joins the pending permission request", async () => {
  const attempt = { hasRequested: false };
  let permissionStateChecks = 0;
  let requestCalls = 0;
  let releaseRequest;
  const request = new Promise((resolve) => {
    releaseRequest = resolve;
  });
  const getPermissionState = async () => {
    permissionStateChecks++;
    return "default";
  };
  const requestAccess = () => {
    requestCalls++;
    return request;
  };
  const setDesktopEnabled = async () => true;

  const first = ensureFeedNotificationPermission(
    attempt,
    setDesktopEnabled,
    getPermissionState,
    requestAccess,
  );
  await Promise.resolve();
  const second = ensureFeedNotificationPermission(
    attempt,
    setDesktopEnabled,
    getPermissionState,
    requestAccess,
  );

  assert.equal(permissionStateChecks, 1);
  assert.equal(requestCalls, 2);
  releaseRequest("granted");
  assert.deepEqual(await Promise.all([first, second]), [true, true]);
});
