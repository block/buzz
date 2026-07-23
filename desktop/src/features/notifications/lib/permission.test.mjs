import assert from "node:assert/strict";
import test from "node:test";

import { ensureDesktopNotificationPermission } from "./permission.ts";

test("Windows retries a false denied permission and accepts the granted result", async () => {
  let requestCount = 0;

  const permission = await ensureDesktopNotificationPermission({
    currentPermission: "denied",
    isWindows: true,
    requestAccess: async () => {
      requestCount += 1;
      return "granted";
    },
  });

  assert.equal(permission, "granted");
  assert.equal(requestCount, 1);
});

test("non-Windows platforms keep denied permission without requesting again", async () => {
  let requestCount = 0;

  const permission = await ensureDesktopNotificationPermission({
    currentPermission: "denied",
    isWindows: false,
    requestAccess: async () => {
      requestCount += 1;
      return "granted";
    },
  });

  assert.equal(permission, "denied");
  assert.equal(requestCount, 0);
});

test("default permission still requests access on every platform", async () => {
  for (const isWindows of [false, true]) {
    let requestCount = 0;

    const permission = await ensureDesktopNotificationPermission({
      currentPermission: "default",
      isWindows,
      requestAccess: async () => {
        requestCount += 1;
        return "granted";
      },
    });

    assert.equal(permission, "granted");
    assert.equal(requestCount, 1);
  }
});
