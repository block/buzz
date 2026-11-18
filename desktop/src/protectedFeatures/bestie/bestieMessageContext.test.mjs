import assert from "node:assert/strict";
import test from "node:test";

import { buildBestieMessageContext } from "./bestieMessageContext.ts";

const CHANNEL_ID = "channel-1";

function message(overrides = {}) {
  return {
    id: "message-1",
    author: "Baxen",
    body: "This visible body must not be copied into the agent prompt.",
    createdAt: 1,
    depth: 0,
    time: "8:52 AM",
    ...overrides,
  };
}

test("sends a canonical link to a top-level message instead of its body", () => {
  const context = buildBestieMessageContext(CHANNEL_ID, message());

  assert.equal(
    context,
    "Help me with this thread from Baxen:\n\nbuzz://message?channel=channel-1&id=message-1",
  );
  assert.doesNotMatch(context, /visible body/);
});

test("links a reply to its thread root", () => {
  const context = buildBestieMessageContext(
    CHANNEL_ID,
    message({ id: "reply-1", rootId: "root-1", parentId: "root-1" }),
  );

  assert.equal(
    context,
    "Help me with this thread from Baxen:\n\nbuzz://message?channel=channel-1&id=root-1&thread=root-1",
  );
});

test("does not construct a lossy fallback when channel context is missing", () => {
  assert.equal(buildBestieMessageContext(null, message()), null);
  assert.equal(buildBestieMessageContext(CHANNEL_ID, undefined), null);
});
