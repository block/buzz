import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveShellRoute,
  shouldBounceForChannelNotification,
} from "./AppShell.helpers.ts";

test("deriveShellRoute_selectsCommandConsoleForConsolePath", () => {
  assert.deepEqual(deriveShellRoute("/console"), {
    selectedChannelId: null,
    selectedView: "console",
  });
});

test("battle rhythm route selects its own sidebar destination", () => {
  assert.deepEqual(deriveShellRoute("/battle-rhythm"), {
    selectedChannelId: null,
    selectedView: "battleRhythm",
  });
});

test("planning routes select Plans without selecting developer Projects", () => {
  assert.deepEqual(deriveShellRoute("/plans"), {
    selectedChannelId: null,
    selectedView: "plans",
  });
  assert.deepEqual(deriveShellRoute("/plans/deployment-1"), {
    selectedChannelId: null,
    selectedView: "plans",
  });
});

test("shouldBounceForChannelNotification_allowsTopLevelChannelMessages", () => {
  assert.equal(shouldBounceForChannelNotification([["h", "channel"]]), true);
});

test("shouldBounceForChannelNotification_suppressesThreadReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
    ]),
    false,
  );
});

test("shouldBounceForChannelNotification_allowsBroadcastReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ]),
    true,
  );
});
