import assert from "node:assert/strict";
import test from "node:test";

import { isDmNotifiableKind } from "./isDmNotifiableKind.ts";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";

// Regression guard for the phantom-DM-notification bug: when kind:5 deletes
// gained an `h` tag, they started matching the live DM subscription. Without
// this gate, deleting a DM message fires a "New message" toast on the other
// side. Reactions (7), Buzz-native deletes (9005), edits (40003), diffs
// (40008), and system messages (40099) hit the same subscription and must
// also be filtered.

test("human-visible message kinds fire DM notifications", () => {
  assert.equal(isDmNotifiableKind(9), true, "kind:9 stream message");
  assert.equal(isDmNotifiableKind(40002), true, "kind:40002 stream message v2");
  assert.equal(isDmNotifiableKind(45001), true, "kind:45001 forum post");
  assert.equal(isDmNotifiableKind(45003), true, "kind:45003 forum comment");
  assert.equal(
    isDmNotifiableKind(KIND_HUDDLE_STARTED),
    true,
    "kind:48100 huddle start invite",
  );
});

test("non-message kinds do NOT fire DM notifications", () => {
  assert.equal(isDmNotifiableKind(5), false, "kind:5 NIP-09 deletion");
  assert.equal(isDmNotifiableKind(7), false, "kind:7 reaction");
  assert.equal(isDmNotifiableKind(9005), false, "kind:9005 Buzz-native delete");
  assert.equal(isDmNotifiableKind(40003), false, "kind:40003 message edit");
  assert.equal(isDmNotifiableKind(40008), false, "kind:40008 message diff");
  assert.equal(isDmNotifiableKind(40099), false, "kind:40099 system message");
  assert.equal(
    isDmNotifiableKind(KIND_HUDDLE_PARTICIPANT_JOINED),
    false,
    "kind:48101 huddle participant joined",
  );
  assert.equal(
    isDmNotifiableKind(KIND_HUDDLE_PARTICIPANT_LEFT),
    false,
    "kind:48102 huddle participant left",
  );
  assert.equal(
    isDmNotifiableKind(KIND_HUDDLE_ENDED),
    false,
    "kind:48103 huddle ended",
  );
});
