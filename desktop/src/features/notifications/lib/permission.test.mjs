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
