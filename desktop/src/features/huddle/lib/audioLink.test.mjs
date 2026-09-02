import assert from "node:assert/strict";
import test from "node:test";

import { audioLinkNotice } from "./audioLink.ts";

test("live audio shows no notice", () => {
  assert.equal(audioLinkNotice({ status: "live" }), null);
  assert.equal(audioLinkNotice(undefined), null);
});

test("reconnecting is informational and names a relay restart when draining", () => {
  assert.deepEqual(
    audioLinkNotice({ status: "reconnecting", attempt: 3, draining: false }),
    { tone: "info", message: "Audio dropped. Reconnecting…" },
  );
  assert.deepEqual(
    audioLinkNotice({ status: "reconnecting", attempt: 1, draining: true }),
    { tone: "info", message: "The huddle relay is restarting. Reconnecting…" },
  );
});

test("a lost link is an error that tells the user what to do", () => {
  assert.deepEqual(audioLinkNotice({ status: "lost" }), {
    tone: "error",
    message: "Couldn’t reconnect audio. Leave and rejoin the huddle.",
  });
});
