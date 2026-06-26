import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAgentConversationLink,
  isAgentConversationLink,
  parseAgentConversationLink,
  resolveAgentConversationLinkRenderTarget,
} from "./agentConversationLink.ts";

const CHANNEL = "f570339f-8f8a-4e08-a779-8d954aa44109";
const REPLY =
  "b04819ffc1f7c8ffb49c6d30b5899f470198264680d02e78894a658e30a9059f";

test("buildAgentConversationLink → parseAgentConversationLink round-trips", () => {
  const url = buildAgentConversationLink({
    agentReplyId: REPLY,
    channelId: CHANNEL,
  });
  assert.equal(url, `buzz://task?channel=${CHANNEL}&reply=${REPLY}`);

  const parsed = parseAgentConversationLink(url);
  assert.equal(parsed.ok, true);
  assert.deepEqual(parsed.ok && parsed.value, {
    agentReplyId: REPLY,
    channelId: CHANNEL,
  });
});

test("buildAgentConversationLink rejects missing required params", () => {
  assert.throws(() =>
    buildAgentConversationLink({ agentReplyId: REPLY, channelId: "" }),
  );
  assert.throws(() =>
    buildAgentConversationLink({ agentReplyId: "", channelId: CHANNEL }),
  );
});

test("parseAgentConversationLink rejects unsupported schemes and hosts", () => {
  assert.deepEqual(
    parseAgentConversationLink(
      `https://example.com/?channel=${CHANNEL}&reply=${REPLY}`,
    ),
    { ok: false, reason: "wrong-scheme" },
  );
  assert.deepEqual(
    parseAgentConversationLink("buzz://message?channel=c&id=m"),
    {
      ok: false,
      reason: "wrong-host",
    },
  );
});

test("parseAgentConversationLink rejects missing required params", () => {
  assert.deepEqual(parseAgentConversationLink(`buzz://task?reply=${REPLY}`), {
    ok: false,
    reason: "missing-channel",
  });
  assert.deepEqual(
    parseAgentConversationLink(`buzz://task?channel=${CHANNEL}`),
    {
      ok: false,
      reason: "missing-reply",
    },
  );
});

test("isAgentConversationLink matches task links only", () => {
  assert.equal(
    isAgentConversationLink(`buzz://task?channel=${CHANNEL}&reply=${REPLY}`),
    true,
  );
  assert.equal(isAgentConversationLink("buzz://message?channel=c&id=m"), false);
  assert.equal(isAgentConversationLink("https://example.com"), false);
  assert.equal(isAgentConversationLink(undefined), false);
  assert.equal(isAgentConversationLink(""), false);
});

test("resolveAgentConversationLinkRenderTarget distinguishes cards from labels", () => {
  const href = `buzz://task?channel=${CHANNEL}&reply=${REPLY}`;
  const link = {
    agentReplyId: REPLY,
    channelId: CHANNEL,
  };

  assert.deepEqual(
    resolveAgentConversationLinkRenderTarget({ href, label: href }),
    {
      kind: "card",
      link,
    },
  );
  assert.deepEqual(
    resolveAgentConversationLinkRenderTarget({ href, label: "task" }),
    {
      kind: "label",
      link,
    },
  );
  assert.deepEqual(
    resolveAgentConversationLinkRenderTarget({
      href: "https://example.com",
      label: href,
    }),
    { kind: "none" },
  );
});
