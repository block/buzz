import assert from "node:assert/strict";
import test from "node:test";

// A fresh macOS install: Desktop alerts default to on in Settings, but the
// native authorization has never been requested (UNAuthorizationStatus
// NotDetermined → "default").
let nativePermission = "default";
let requestResult = "granted";
const calls = [];

globalThis.isTauri = true;
globalThis.window = { Notification: class {} };
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { platform: "MacIntel", userAgent: "Macintosh" },
});
globalThis.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    calls.push({ command, args });
    if (command === "notification_permission_state") {
      return nativePermission;
    }
    if (command === "request_notification_access") {
      nativePermission = requestResult;
      return requestResult;
    }
    if (command === "show_native_notification") {
      return null;
    }
    throw new Error(`unmocked Tauri command: ${command}`);
  },
  transformCallback: () => 1,
};
window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

const { sendDesktopNotification } = await import("./desktop.ts");

function commandsCalled(command) {
  return calls.filter((call) => call.command === command);
}

test("first alert requests the never-asked OS permission once, then delivers", async () => {
  const [first, second] = await Promise.all([
    sendDesktopNotification({ title: "DM", body: "hey" }),
    sendDesktopNotification({ title: "Thread reply", body: "yo" }),
  ]);

  assert.equal(first, true);
  assert.equal(second, true);
  assert.equal(commandsCalled("request_notification_access").length, 1);
  assert.deepEqual(
    commandsCalled("show_native_notification").map((call) => call.args.title),
    ["DM", "Thread reply"],
  );
});

test("does not prompt again later in the same process", async () => {
  // Simulate the user resetting the permission while Buzz keeps running.
  nativePermission = "default";
  requestResult = "denied";

  const delivered = await sendDesktopNotification({ title: "Mention" });

  assert.equal(delivered, false);
  assert.equal(commandsCalled("request_notification_access").length, 1);
});
