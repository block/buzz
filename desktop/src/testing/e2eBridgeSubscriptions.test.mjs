import assert from "node:assert/strict";
import test from "node:test";
import { hasMockSubscription } from "./e2eBridgeSubscriptions.ts";

test("channel readiness rejects global-only, wrong-channel and wrong-kind REQs", () => {
  const subscription = (channelIds, kinds) => ({ channelIds, kinds });
  for (const candidate of [
    subscription(["*"], [9]),
    subscription(["other"], [9]),
    subscription(["channel"], [30078]),
  ]) {
    assert.equal(hasMockSubscription([candidate], "channel", 9, true), false);
  }
  assert.equal(hasMockSubscription([], "channel", 9, true), false);
  for (const kinds of [[9], [7, 9], null]) {
    assert.equal(
      hasMockSubscription(
        [subscription(["channel"], kinds)],
        "channel",
        9,
        true,
      ),
      true,
    );
  }
});

test("legacy readiness and explicit global queries retain their semantics", () => {
  const global = [{ channelIds: ["*"], kinds: [30078] }];
  assert.equal(hasMockSubscription(global, "channel"), true);
  assert.equal(hasMockSubscription(global, "channel", 9), false);
  assert.equal(hasMockSubscription(global, "*", 30078), true);
  assert.equal(hasMockSubscription(global, "*", 9), false);
});
