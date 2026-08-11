import assert from "node:assert/strict";
import test from "node:test";

import {
  exceedsMentionPubkeyLimit,
  MAX_MENTION_PUBKEYS,
} from "./useMentionSendFlow.helpers.ts";

test("mention preflight accepts the maximum number of unique targets", () => {
  const pubkeys = Array.from({ length: MAX_MENTION_PUBKEYS }, (_, index) =>
    index.toString(16).padStart(64, "0"),
  );

  assert.equal(exceedsMentionPubkeyLimit(pubkeys, []), false);
  assert.equal(exceedsMentionPubkeyLimit([...pubkeys, pubkeys[0]], []), false);
});

test("mention preflight counts unresolved persona targets before provisioning", () => {
  const pubkeys = Array.from({ length: MAX_MENTION_PUBKEYS - 1 }, (_, index) =>
    index.toString(16).padStart(64, "0"),
  );

  assert.equal(exceedsMentionPubkeyLimit(pubkeys, ["persona-1"]), false);
  assert.equal(
    exceedsMentionPubkeyLimit(pubkeys, ["persona-1", "persona-2"]),
    true,
  );
});
