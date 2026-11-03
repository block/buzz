import assert from "node:assert/strict";
import test from "node:test";

const notifications = [];

class WorkingNotification {
  static permission = "granted";

  constructor(title, options) {
    notifications.push({ title, options });
  }

  close() {}
}

class ThrowingNotification {
  static permission = "granted";

  constructor() {
    throw new Error("notification backend unavailable");
  }
}

globalThis.window = { Notification: ThrowingNotification };

const { sendDesktopNotification, getDesktopNotificationPermissionState } =
  await import("./desktop.ts");

// Stands in for the plugin's injected shim: `permission` is a cached value and
// only `requestPermission()` reaches the backend and rewrites it.
function shimNotification(initialPermission, grantedPermission = "granted") {
  class ShimNotification {
    static permission = initialPermission;
    static requestCount = 0;

    static async requestPermission() {
      ShimNotification.requestCount += 1;
      ShimNotification.permission = grantedPermission;
      return grantedPermission;
    }

    close() {}
  }

  return ShimNotification;
}

async function withEnvironment({ platform, isTauri, notification }, callback) {
  const originalNavigator = Object.getOwnPropertyDescriptor(
    globalThis,
    "navigator",
  );
  const originalNotification = window.Notification;
  const originalIsTauri = globalThis.isTauri;

  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { platform, userAgent: "" },
  });
  window.Notification = notification;
  globalThis.isTauri = isTauri;

  try {
    return await callback();
  } finally {
    if (originalNavigator) {
      Object.defineProperty(globalThis, "navigator", originalNavigator);
    } else {
      delete globalThis.navigator;
    }
    window.Notification = originalNotification;
    globalThis.isTauri = originalIsTauri;
  }
}

test("constructor failure is a delivery miss and does not prevent a later notification", async (t) => {
  const warnings = [];
  t.mock.method(console, "warn", (...args) => warnings.push(args));

  const failed = await sendDesktopNotification({ title: "First" });

  assert.equal(failed, false);
  assert.equal(warnings.length, 1);
  assert.match(String(warnings[0][1]), /notification backend unavailable/);

  window.Notification = WorkingNotification;

  const delivered = await sendDesktopNotification({
    title: "Second",
    body: "Recovered",
  });

  assert.equal(delivered, true);
  assert.deepEqual(notifications, [
    {
      title: "Second",
      options: { body: "Recovered", silent: true, extra: undefined },
    },
  ]);
});

test("Windows Tauri repairs the shim's false denied state on read", async () => {
  const Notification = shimNotification("denied");

  const permission = await withEnvironment(
    { platform: "Win32", isTauri: true, notification: Notification },
    () => getDesktopNotificationPermissionState(),
  );

  assert.equal(permission, "granted");
  assert.equal(Notification.requestCount, 1);
});

test("the repaired state is cached, so later reads do not request again", async () => {
  const Notification = shimNotification("denied");

  await withEnvironment(
    { platform: "Win32", isTauri: true, notification: Notification },
    async () => {
      await getDesktopNotificationPermissionState();
      const second = await getDesktopNotificationPermissionState();
      assert.equal(second, "granted");
    },
  );

  assert.equal(Notification.requestCount, 1);
});

test("a denial that survives the request is reported as denied", async () => {
  const Notification = shimNotification("denied", "denied");

  const permission = await withEnvironment(
    { platform: "Win32", isTauri: true, notification: Notification },
    () => getDesktopNotificationPermissionState(),
  );

  assert.equal(permission, "denied");
  assert.equal(Notification.requestCount, 1);
});

test("denied stays terminal outside the Windows Tauri app", async () => {
  for (const environment of [
    { label: "Windows web", platform: "Win32", isTauri: false },
    { label: "Linux Tauri", platform: "Linux x86_64", isTauri: true },
    { label: "Linux web", platform: "Linux x86_64", isTauri: false },
  ]) {
    const Notification = shimNotification("denied");

    const permission = await withEnvironment(
      { ...environment, notification: Notification },
      () => getDesktopNotificationPermissionState(),
    );

    assert.equal(permission, "denied", environment.label);
    assert.equal(Notification.requestCount, 0, environment.label);
  }
});

test("granted is returned untouched and never triggers a request", async () => {
  const Notification = shimNotification("granted");

  const permission = await withEnvironment(
    { platform: "Win32", isTauri: true, notification: Notification },
    () => getDesktopNotificationPermissionState(),
  );

  assert.equal(permission, "granted");
  assert.equal(Notification.requestCount, 0);
});
