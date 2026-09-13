import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    localStorage: dom.window.localStorage,
  });
});
after(() => dom.window.close());

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
      return "granted";
    },
    async (item) => {
      delivered.push(item.id);
      return true;
    },
  );

  await Promise.resolve();
  assert.deepEqual(delivered, []);

  releaseRepair();
  assert.deepEqual(await delivery, {
    handledIds: ["restart-alert"],
    retryableIds: [],
  });
  assert.deepEqual(delivered, ["restart-alert"]);
});

test("a permission repair that is not granted suppresses the feed batch", async () => {
  const delivered = [];

  const result = await deliverFeedNotificationBatch(
    [{ id: "blocked-alert" }],
    async () => "denied",
    async (item) => {
      delivered.push(item.id);
      return true;
    },
  );

  assert.deepEqual(delivered, []);
  assert.deepEqual(result, {
    handledIds: ["blocked-alert"],
    retryableIds: [],
  });
});

test("an operational permission failure keeps the feed batch retryable", async () => {
  let delivered = false;
  const result = await deliverFeedNotificationBatch(
    [{ id: "retry-permission-alert" }],
    async () => "error",
    async () => {
      delivered = true;
      return true;
    },
  );

  assert.equal(delivered, false);
  assert.deepEqual(result, {
    handledIds: [],
    retryableIds: ["retry-permission-alert"],
  });
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
  assert.deepEqual(await Promise.all([first, second]), ["granted", "granted"]);
});

test("a delivery failure remains unseen and retries on the next feed result", async (t) => {
  t.mock.method(console, "warn", () => {});
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useFeedDesktopNotifications } = await import(
    "./use-feed-desktop-notifications.ts"
  );
  let shouldFail = true;
  const delivered = [];
  class TestNotification {
    static permission = "granted";

    constructor(_title, options) {
      if (shouldFail) {
        throw new Error("notification backend unavailable");
      }
      delivered.push(options.extra.buzzNotificationTarget.eventId);
    }

    close() {}
  }
  Object.defineProperty(window, "Notification", {
    configurable: true,
    value: TestNotification,
  });
  const emptyFeed = { feed: { mentions: [], needsAction: [] } };
  const item = {
    id: "retry-alert",
    kind: 9,
    pubkey: "sender",
    content: "retry me",
    createdAt: 123,
    channelId: "channel-id",
    channelName: "ship-room",
    channelType: "stream",
    tags: [],
    category: "mention",
  };
  const settings = {
    desktopEnabled: true,
    slotAlertsEnabled: { mention: true, needs_action: true },
  };
  const setDesktopEnabled = async () => true;
  const profiles = new Map();
  const channels = [
    { id: "channel-id", name: "ship-room", channelType: "stream" },
  ];
  const silentChannelIds = new Set(["channel-id"]);
  const render = (feed) =>
    useFeedDesktopNotifications(
      feed,
      "viewer",
      settings,
      setDesktopEnabled,
      true,
      profiles,
      undefined,
      channels,
      silentChannelIds,
    );
  const hook = renderHook(({ feed }) => render(feed), {
    initialProps: { feed: emptyFeed },
  });
  const settle = () =>
    act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  await settle();

  hook.rerender({ feed: { feed: { mentions: [item], needsAction: [] } } });
  await settle();
  assert.deepEqual(
    JSON.parse(localStorage.getItem("buzz-home-feed-seen.v1:viewer")),
    [],
  );

  shouldFail = false;
  hook.rerender({ feed: { feed: { mentions: [item], needsAction: [] } } });
  await settle();
  assert.deepEqual(delivered, ["retry-alert"]);
  assert.deepEqual(
    JSON.parse(localStorage.getItem("buzz-home-feed-seen.v1:viewer")),
    ["retry-alert"],
  );

  hook.unmount();
  cleanup();
});
