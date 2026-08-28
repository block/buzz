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
  throw new Error("WebView2 notification constructor must not run on Windows");
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
  value: { platform: "Win32", userAgent: "buzz-test" },
});

const {
  getDesktopNotificationPermissionState,
  requestDesktopNotificationAccess,
  sendDesktopNotification,
} = await import("./desktop.ts");

test("Windows Tauri treats native WinRT as granted even when WebView2 is denied", async () => {
  assert.equal(await getDesktopNotificationPermissionState(), "granted");
  assert.equal(await requestDesktopNotificationAccess(), "granted");
});

test("Windows Tauri posts native notifications without the WebView2 constructor", async () => {
  invokes.length = 0;
  const delivered = await sendDesktopNotification({
    title: "Taylor in #general",
    body: "enene",
    target: { channelId: "ch-1", eventId: "ev-1", kind: 9 },
  });
  assert.equal(delivered, true);
  assert.equal(invokes.length, 1);
  assert.equal(invokes[0].command, "show_native_notification");
  assert.equal(invokes[0].payload.title, "Taylor in #general");
});
