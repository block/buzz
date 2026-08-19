import assert from "node:assert/strict";
import test from "node:test";

const notifications = [];

class WorkingNotification {
  static permission = "granted";

  constructor(title, options) {
    notifications.push({ title, options });
  }

  close() {}
}

class ThrowingNotification {
  static permission = "granted";

  constructor() {
    throw new Error("notification backend unavailable");
  }
}

globalThis.window = { Notification: ThrowingNotification };

const { sendDesktopNotification } = await import("./desktop.ts");

test("constructor failure is a delivery miss and does not prevent a later notification", async (t) => {
  const warnings = [];
  t.mock.method(console, "warn", (...args) => warnings.push(args));

  const failed = await sendDesktopNotification({ title: "First" });

  assert.equal(failed, false);
  assert.equal(warnings.length, 1);
  assert.match(String(warnings[0][1]), /notification backend unavailable/);

  window.Notification = WorkingNotification;

  const delivered = await sendDesktopNotification({
    title: "Second",
    body: "Recovered",
  });

  assert.equal(delivered, true);
  assert.deepEqual(notifications, [
    {
      title: "Second",
      options: { body: "Recovered", silent: true, extra: undefined },
    },
  ]);
});
