import assert from "node:assert/strict";
import test from "node:test";

import { fromRawCodexSharedRuntimeStatus } from "./codexTaskTypes.ts";

test("maps shared-runtime process diagnostics from Tauri", () => {
  assert.deepEqual(
    fromRawCodexSharedRuntimeStatus({
      enabled: true,
      state: "ready",
      url: "ws://127.0.0.1:51919",
      detail: null,
      desktop_process_ids: [10, 11],
      private_app_server_process_ids: [12],
      desktop_detection_error: null,
    }),
    {
      enabled: true,
      state: "ready",
      url: "ws://127.0.0.1:51919",
      detail: null,
      desktopProcessIds: [10, 11],
      privateAppServerProcessIds: [12],
      desktopDetectionError: null,
    },
  );
});

test("older backends map missing process fields to safe defaults", () => {
  const status = fromRawCodexSharedRuntimeStatus({
    enabled: false,
    state: "setup_required",
    url: "ws://127.0.0.1:51919",
  });
  assert.deepEqual(status.desktopProcessIds, []);
  assert.deepEqual(status.privateAppServerProcessIds, []);
  assert.equal(status.desktopDetectionError, null);
});
