import assert from "node:assert/strict";
import test from "node:test";
import { classifyPushEnv, pushPrefsFromSettings } from "./env.ts";

const base = {
  hasSw: true,
  hasPush: true,
  hasNotification: true,
  ios: false,
  standalone: false,
  permission: "default",
};

test("classifyPushEnv", () => {
  assert.equal(
    classifyPushEnv({ ...base, ios: true, hasPush: false }),
    "ios-install",
  );
  assert.equal(
    classifyPushEnv({ ...base, ios: true, standalone: true }),
    "needs-tap",
  );
  assert.equal(classifyPushEnv({ ...base, hasPush: false }), "unsupported");
  assert.equal(classifyPushEnv({ ...base, permission: "denied" }), "denied");
  assert.equal(classifyPushEnv({ ...base, permission: "granted" }), "ready");
  assert.equal(classifyPushEnv(base), "needs-tap");
});

test("pushPrefsFromSettings", () => {
  assert.deepEqual(pushPrefsFromSettings(null), {
    dm: true,
    mention: true,
    thread_reply: true,
    needs_action: true,
  });
  assert.deepEqual(
    pushPrefsFromSettings(
      JSON.stringify({
        desktopEnabled: true,
        slotAlertsEnabled: { dm: false },
      }),
    ),
    { dm: false, mention: true, thread_reply: true, needs_action: true },
  );
  assert.equal(
    pushPrefsFromSettings(JSON.stringify({ desktopEnabled: false })).mention,
    false,
  );
  assert.equal(pushPrefsFromSettings("{bad").dm, true);
});
