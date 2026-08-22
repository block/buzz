import assert from "node:assert/strict";
import test from "node:test";

import { ensureDesktopNotificationPermission } from "./permission.ts";

test("Windows Tauri retries a false denied permission and accepts the granted result", async () => {
  let requestCount = 0;

  const permission = await ensureDesktopNotificationPermission({
    currentPermission: "denied",
    isWindowsTauri: true,
    requestAccess: async () => {
      requestCount += 1;
      return "granted";
    },
  });

  assert.equal(permission, "granted");
  assert.equal(requestCount, 1);
});

test("non-Windows-Tauri environments keep denied permission without requesting again", async () => {
  for (const environment of [
    "Windows web",
    "non-Windows Tauri",
    "non-Windows web",
  ]) {
    let requestCount = 0;

    const permission = await ensureDesktopNotificationPermission({
      currentPermission: "denied",
      isWindowsTauri: false,
      requestAccess: async () => {
        requestCount += 1;
        return "granted";
      },
    });

    assert.equal(permission, "denied", environment);
    assert.equal(requestCount, 0, environment);
  }
});

test("default permission still requests access on every platform", async () => {
  for (const isWindowsTauri of [false, true]) {
    let requestCount = 0;

    const permission = await ensureDesktopNotificationPermission({
      currentPermission: "default",
      isWindowsTauri,
      requestAccess: async () => {
        requestCount += 1;
        return "granted";
      },
    });

    assert.equal(permission, "granted");
    assert.equal(requestCount, 1);
  }
});

test("Windows Tauri recovers a false denied at boot so persisted desktopEnabled survives relaunch", async () => {
  // Simulates the mount-time refreshPermission path: the init shim stamps
  // "denied" before the app is registered as a notification sender, but a
  // single requestPermission() returns "granted". Without this recovery the
  // mount-time effect writes desktopEnabled=false before the user touches
  // anything, requiring re-enabling after every restart.
  let requestCount = 0;

  const permission = await ensureDesktopNotificationPermission({
    currentPermission: "denied",
    isWindowsTauri: true,
    requestAccess: async () => {
      requestCount += 1;
      return "granted";
    },
  });

  assert.equal(permission, "granted");
  assert.equal(requestCount, 1, "boot-time recovery fires exactly once");
});

test("Windows Tauri does not recover a genuine granted permission at boot", async () => {
  // If the OS already grants permission, the boot-time read should not
  // trigger an unnecessary requestPermission() call.
  let requestCount = 0;

  const permission = await ensureDesktopNotificationPermission({
    currentPermission: "granted",
    isWindowsTauri: true,
    requestAccess: async () => {
      requestCount += 1;
      return "granted";
    },
  });

  assert.equal(permission, "granted");
  assert.equal(requestCount, 0, "granted does not trigger a re-request");
});
