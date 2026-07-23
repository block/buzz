import assert from "node:assert/strict";
import test from "node:test";

import { collectThreadAuxMessageIds } from "./useThreadReplies.ts";

const ROOT_ID = "1".repeat(64);
const REPLY_ID = "2".repeat(64);

function reply(id = REPLY_ID) {
  return {
    id,
    pubkey: "a".repeat(64),
    kind: 9,
    created_at: 1_700_000_000,
    content: "reply",
    tags: [["e", ROOT_ID]],
    sig: "sig",
  };
}

test("thread aux hydration includes the root when there are no replies", () => {
  assert.deepEqual(collectThreadAuxMessageIds(ROOT_ID, []), [ROOT_ID]);
});

test("thread aux hydration includes and deduplicates root and reply ids", () => {
  assert.deepEqual(
    collectThreadAuxMessageIds(ROOT_ID, [reply(), reply(ROOT_ID)]),
    [ROOT_ID, REPLY_ID],
  );
});
