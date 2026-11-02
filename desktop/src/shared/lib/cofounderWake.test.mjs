import assert from "node:assert/strict";
import test from "node:test";

import {
  COFOUNDER_WAKE_TAG,
  isCofounderWakeEvent,
  withoutCofounderWakes,
} from "./cofounderWake.ts";

const RELAY = "ab".repeat(32);
const MEMBER = "cd".repeat(32);

const wake = (pubkey, kind = 9) => ({
  kind,
  pubkey,
  tags: [
    ["h", "dm"],
    [COFOUNDER_WAKE_TAG, "mode_on"],
  ],
});

test("only the relay's own tagged stream message is a wake", () => {
  assert.equal(isCofounderWakeEvent(wake(RELAY), RELAY), true);
  assert.equal(isCofounderWakeEvent(wake(RELAY, 40002), RELAY), true);
  assert.equal(
    isCofounderWakeEvent(wake(RELAY.toUpperCase()), RELAY),
    true,
    "key case does not matter",
  );
  assert.equal(
    isCofounderWakeEvent(wake(MEMBER), RELAY),
    false,
    "a member copying the tag is not hidden",
  );
  assert.equal(
    isCofounderWakeEvent(
      { kind: 9, pubkey: RELAY, tags: [["h", "dm"]] },
      RELAY,
    ),
    false,
  );
  assert.equal(isCofounderWakeEvent(wake(RELAY, 1), RELAY), false);
});

test("an unknown relay key fails closed for a tagged message only", () => {
  assert.equal(isCofounderWakeEvent(wake(MEMBER), undefined), true);
  assert.equal(isCofounderWakeEvent(wake(MEMBER), null), true);
  assert.equal(
    isCofounderWakeEvent({ kind: 9, pubkey: MEMBER, tags: [] }, undefined),
    false,
  );
});

test("a list is only filtered, and the relay key only read, when it must be", async () => {
  let reads = 0;
  const read = async () => {
    reads += 1;
    return RELAY;
  };
  const plain = [{ kind: 9, pubkey: MEMBER, tags: [] }];
  assert.deepEqual(await withoutCofounderWakes(plain, read), plain);
  assert.equal(reads, 0, "no tagged item, no lookup");

  const mixed = [wake(RELAY), wake(MEMBER), plain[0]];
  const kept = await withoutCofounderWakes(mixed, read);
  assert.deepEqual(kept, [mixed[1], mixed[2]]);
  assert.equal(reads, 1);

  const failed = await withoutCofounderWakes(mixed, async () => {
    throw new Error("offline");
  });
  assert.deepEqual(failed, [mixed[2]], "an unreadable key fails closed");
});
