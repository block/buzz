import assert from "node:assert/strict";
import test from "node:test";

let nativePermission = "denied";
let nativeQueries = 0;
let nativePermissionError = false;
const testWindow = {
  Notification: Object.assign(function StubNotification() {}, {
    permission: "denied",
  }),
  __TAURI_INTERNALS__: {
    invoke(command) {
      if (command === "windows_notification_permission_state") {
        nativeQueries++;
        if (nativePermissionError) {
          return Promise.reject(new Error("Windows permission query failed"));
        }
        return Promise.resolve(nativePermission);
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    },
  },
};
globalThis.window = testWindow;
globalThis.isTauri = true;
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { platform: "Win32", userAgent: "buzz-test" },
});

const {
  ensureDesktopNotificationPermissionGranted,
  getDesktopNotificationPermissionState,
  requestDesktopNotificationAccess,
} = await import("./desktop.ts");

test("Windows reports a terminal native denial without requesting again", async () => {
  let requested = false;
  assert.equal(await getDesktopNotificationPermissionState(), "denied");
  assert.equal(
    await ensureDesktopNotificationPermissionGranted(
      getDesktopNotificationPermissionState,
      async () => {
        requested = true;
        return "granted";
      },
    ),
    false,
  );
  assert.equal(requested, false);
});

test("Windows access requests re-query the native setting", async () => {
  nativePermission = "granted";
  const before = nativeQueries;
  assert.equal(await requestDesktopNotificationAccess(), "granted");
  assert.equal(nativeQueries, before + 1);
});

test("Windows delivery returns false when the native permission query fails", async (t) => {
  const { sendDesktopNotification } = await import("./desktop.ts");
  t.mock.method(console, "warn", () => {});
  nativePermissionError = true;

  assert.equal(await sendDesktopNotification({ title: "Unavailable" }), false);

  nativePermissionError = false;
});
