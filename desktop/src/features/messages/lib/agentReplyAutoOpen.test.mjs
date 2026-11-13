import assert from "node:assert/strict";
import test from "node:test";

import { decideAgentReplyAutoOpen } from "./agentReplyAutoOpen.ts";

const ROOT_ID = "1".repeat(64);
const OTHER_ROOT_ID = "2".repeat(64);
const REPLY_ID = "3".repeat(64);
const AGENT_PUBKEY = "4".repeat(64);
const HUMAN_PUBKEY = "5".repeat(64);

function event(overrides = {}) {
  return {
    id: overrides.id ?? REPLY_ID,
    pubkey: overrides.pubkey ?? AGENT_PUBKEY,
    created_at: 100,
    kind: 9,
    tags: overrides.tags ?? [["e", ROOT_ID, "", "reply"]],
    content: "hi",
    sig: "0".repeat(128),
    ...overrides,
  };
}

function decide(overrides = {}) {
  return decideAgentReplyAutoOpen({
    event: event(),
    hasActiveAuxiliaryPanel: false,
    expectedRootId: ROOT_ID,
    agentPubkeys: new Set([AGENT_PUBKEY]),
    ...overrides,
  });
}

test("opens when a live agent reply targets the expected root", () => {
  assert.deepEqual(decide(), { rootId: ROOT_ID, replyId: REPLY_ID });
});

test("skips when an auxiliary panel is already open", () => {
  assert.deepEqual(decide({ hasActiveAuxiliaryPanel: true }), {});
});

test("skips when no expected root id is pending", () => {
  assert.deepEqual(decide({ expectedRootId: null }), {});
});

test("skips non-message events with forged reply tags", () => {
  assert.deepEqual(decide({ event: event({ kind: 40003 }) }), {});
});

test("skips human replies", () => {
  assert.deepEqual(decide({ event: event({ pubkey: HUMAN_PUBKEY }) }), {});
});

test("skips top-level messages", () => {
  assert.deepEqual(decide({ event: event({ tags: [] }) }), {});
});

test("skips broadcast replies", () => {
  assert.deepEqual(
    decide({
      event: event({
        tags: [
          ["e", ROOT_ID, "", "reply"],
          ["broadcast", "1"],
        ],
      }),
    }),
    {},
  );
});

test("skips replies to a different thread root", () => {
  assert.deepEqual(
    decide({
      event: event({ tags: [["e", OTHER_ROOT_ID, "", "reply"]] }),
    }),
    {},
  );
});

test("resolves the root marker for nested replies", () => {
  assert.deepEqual(
    decide({
      event: event({
        tags: [
          ["e", ROOT_ID, "", "root"],
          ["e", OTHER_ROOT_ID, "", "reply"],
        ],
      }),
    }),
    { rootId: ROOT_ID, replyId: REPLY_ID },
  );
});
