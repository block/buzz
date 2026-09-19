import assert from "node:assert/strict";
import test from "node:test";

const invokes = [];
let nextInvokeError = null;

const testWindow = new EventTarget();
testWindow.__TAURI_INTERNALS__ = {
  invoke(command, args) {
    invokes.push({ command, args });
    if (nextInvokeError) {
      const error = nextInvokeError;
      nextInvokeError = null;
      return Promise.reject(error);
    }
    return Promise.resolve(undefined);
  },
  transformCallback() {
    return 0;
  },
  metadata: { currentWindow: { label: "main" } },
};
// Reproduce the notification plugin's Windows initialization race: the shim
// can expose `denied` even though desktop permission is natively granted.
function DeniedNotificationShim() {}
DeniedNotificationShim.permission = "denied";
testWindow.Notification = DeniedNotificationShim;

globalThis.window = testWindow;
globalThis.document = { hasFocus: () => false };
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

test("Windows uses native permission instead of the denied WebView shim", async () => {
  assert.equal(await getDesktopNotificationPermissionState(), "granted");
  assert.equal(await requestDesktopNotificationAccess(), "granted");
});

test("Windows sends through the awaited native notification command", async () => {
  const target = {
    channelId: "channel-1",
    eventId: "event-1",
    kind: 9,
  };

  const delivered = await sendDesktopNotification({
    title: "New message",
    body: "Hello from Buzz",
    target,
  });

  assert.equal(delivered, true);
  assert.deepEqual(invokes, [
    {
      command: "show_native_notification",
      args: {
        title: "New message",
        body: "Hello from Buzz",
        target,
      },
    },
  ]);
});

test("Windows reports native notification delivery failures", async () => {
  nextInvokeError = new Error("Windows toast registration is unavailable");

  assert.equal(
    await sendDesktopNotification({ title: "Undeliverable message" }),
    false,
  );
});
