import assert from "node:assert/strict";
import { afterEach, describe, it, mock } from "node:test";

import { startReminderNotificationPoll } from "./reminderNotificationPoll.ts";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;

afterEach(() => {
  mock.timers.reset();
  if (originalDocument === undefined) delete globalThis.document;
  else globalThis.document = originalDocument;
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
});

describe("reminder notification polling", () => {
  it("keeps checking while the document is hidden and app is unfocused", () => {
    mock.timers.enable({ apis: ["setInterval"] });
    globalThis.document = {
      visibilityState: "hidden",
      hasFocus: () => false,
    };
    globalThis.window = {
      setInterval,
      clearInterval,
    };

    let checks = 0;
    const stop = startReminderNotificationPoll(() => {
      checks += 1;
    });

    assert.equal(checks, 1);
    mock.timers.tick(30_000);
    assert.equal(checks, 2);
    stop();
    mock.timers.tick(30_000);
    assert.equal(checks, 2);
  });
});
