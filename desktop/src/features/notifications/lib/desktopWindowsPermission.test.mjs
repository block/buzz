import assert from "node:assert/strict";
import test from "node:test";

// Mirrors the `window.Notification` shim that tauri-plugin-notification's
// init script installs. On Windows the shim's own `isPermissionGranted()`
// compares its freshly initialised "default" against "granted" instead of
// asking the backend, so it stamps "denied" on every launch
// (block/buzz#2445). `requestPermission()` is the only path that writes the
// backend's real answer ("granted" on desktop) back into the shim.
function installShim({ permission }) {
  let current = permission;
  const Shim = function ShimNotification() {};
  Shim.requestPermission = async () => {
    current = "granted";
    return "granted";
  };
  Object.defineProperty(Shim, "permission", {
    enumerable: true,
    get: () => current,
  });
  return Shim;
}

function setPlatform(platform) {
  // Node ships a getter-only global `navigator`; redefine it per test.
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { platform, userAgent: `Mozilla/5.0 (${platform})` },
  });
}

// `isTauri()` from @tauri-apps/api/core reads `globalThis.isTauri`.
globalThis.isTauri = true;
globalThis.window = { Notification: installShim({ permission: "denied" }) };
setPlatform("Win32");

const {
  getDesktopNotificationPermissionState,
  requestDesktopNotificationAccess,
} = await import("./desktop.ts");

test("Windows: the shim's startup 'denied' is reported as 'default' so permission can still be requested", async () => {
  setPlatform("Win32");
  window.Notification = installShim({ permission: "denied" });

  assert.equal(await getDesktopNotificationPermissionState(), "default");

  // Requesting flips the shim to the backend's real answer, and from then on
  // the state is authoritative.
  assert.equal(await requestDesktopNotificationAccess(), "granted");
  assert.equal(await getDesktopNotificationPermissionState(), "granted");
});

test("Windows: a real 'granted' passes through unchanged", async () => {
  setPlatform("Win32");
  window.Notification = installShim({ permission: "granted" });

  assert.equal(await getDesktopNotificationPermissionState(), "granted");
});

test("Linux: 'denied' stays authoritative — the placeholder rewrite is Windows-only", async () => {
  setPlatform("Linux x86_64");
  window.Notification = installShim({ permission: "denied" });

  assert.equal(await getDesktopNotificationPermissionState(), "denied");
});

test("browser (non-Tauri) on Windows: 'denied' is a real user decision and stays 'denied'", async () => {
  setPlatform("Win32");
  globalThis.isTauri = false;
  try {
    window.Notification = installShim({ permission: "denied" });
    assert.equal(await getDesktopNotificationPermissionState(), "denied");
  } finally {
    globalThis.isTauri = true;
  }
});
