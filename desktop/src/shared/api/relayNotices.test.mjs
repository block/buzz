import assert from "node:assert/strict";
import test from "node:test";

import {
  clearRelayNoticeListenersForTests,
  emitRelayNotice,
  handleRelayNoticeFrame,
  relayNoticeMessageFromFrame,
  subscribeToRelayNotices,
} from "./relayNotices.ts";

test.afterEach(() => {
  clearRelayNoticeListenersForTests();
});

test("relayNoticeMessageFromFrame parses NOTICE messages only", () => {
  assert.equal(relayNoticeMessageFromFrame(["NOTICE", "hello"]), "hello");
  assert.equal(relayNoticeMessageFromFrame(["NOTICE", "   "]), null);
  assert.equal(relayNoticeMessageFromFrame(["OK", "event", true, ""]), null);
  assert.equal(relayNoticeMessageFromFrame({ type: "NOTICE" }), null);
});

test("handleRelayNoticeFrame consumes NOTICE frames without treating them as errors", () => {
  const messages = [];

  assert.equal(
    handleRelayNoticeFrame(["NOTICE", "offline warning"], (message) => {
      messages.push(message);
    }),
    true,
  );
  assert.deepEqual(messages, ["offline warning"]);

  assert.equal(handleRelayNoticeFrame(["NOTICE", "   "], messages.push), true);
  assert.deepEqual(messages, ["offline warning"]);
  assert.equal(
    handleRelayNoticeFrame(["EOSE", "sub-id"], messages.push),
    false,
  );
});

test("emitRelayNotice notifies subscribers and respects unsubscribe", () => {
  const messages = [];
  const unsubscribe = subscribeToRelayNotices((message) => {
    messages.push(message);
  });

  emitRelayNotice("first");
  unsubscribe();
  emitRelayNotice("second");
  emitRelayNotice("   ");

  assert.deepEqual(messages, ["first"]);
});
