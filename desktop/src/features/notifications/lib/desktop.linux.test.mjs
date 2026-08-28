import assert from "node:assert/strict";
import test from "node:test";

const invokes = [];

const tauriInternals = {
  invoke(command, payload) {
    invokes.push({ command, payload });
    if (command === "show_native_notification") {
      return Promise.resolve();
    }
    return Promise.resolve(undefined);
  },
  transformCallback() {
    return 0;
  },
  metadata: { currentWindow: { label: "main" } },
};

function DeniedNotification() {
  throw new Error("WebKit notification constructor must not run on Linux");
}
DeniedNotification.permission = "denied";

const testWindow = new EventTarget();
testWindow.__TAURI_INTERNALS__ = tauriInternals;
testWindow.Notification = DeniedNotification;
globalThis.window = testWindow;
globalThis.document = new EventTarget();
globalThis.isTauri = true;
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { platform: "Linux x86_64", userAgent: "buzz-test" },
});

const {
  getDesktopNotificationPermissionState,
  requestDesktopNotificationAccess,
  sendDesktopNotification,
} = await import("./desktop.ts");

test("Linux Tauri treats native D-Bus as granted even when WebKit is denied", async () => {
  assert.equal(await getDesktopNotificationPermissionState(), "granted");
  assert.equal(await requestDesktopNotificationAccess(), "granted");
});

test("Linux Tauri posts native notifications without the WebKit constructor", async () => {
  invokes.length = 0;
  const delivered = await sendDesktopNotification({
    title: "Mention",
    body: "hello",
    target: { channelId: "ch-1", eventId: "ev-1", kind: 9 },
  });
  assert.equal(delivered, true);
  assert.equal(invokes.length, 1);
  assert.equal(invokes[0].command, "show_native_notification");
  assert.equal(invokes[0].payload.title, "Mention");
});
