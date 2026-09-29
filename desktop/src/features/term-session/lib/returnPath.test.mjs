import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTermSessionReturnPathSection,
  formatReturnPathMention,
  resolveTermSessionReturnPathMention,
  resolveTermSessionReturnPathOrigin,
} from "./returnPath.ts";

const CHANNEL = "f570339f-8f8a-4e08-a779-8d954aa44109";
const MESSAGE =
  "b04819ffc1f7c8ffb49c6d30b5899f470198264680d02e78894a658e30a9059f";
const THREAD =
  "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
const OTHER_THREAD =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

test("formatReturnPathMention adds @ once", () => {
  assert.equal(formatReturnPathMention("Fable"), "@Fable");
  assert.equal(formatReturnPathMention("@Fable"), "@Fable");
  assert.equal(formatReturnPathMention("  "), "");
});

test("resolveTermSessionReturnPathMention prefers explicit then agent then assistant", () => {
  assert.equal(
    resolveTermSessionReturnPathMention({
      explicitMention: "@Ship",
      agentDisplayName: "Fable",
      channelAssistantName: "Bot",
    }),
    "@Ship",
  );
  assert.equal(
    resolveTermSessionReturnPathMention({
      agentDisplayName: "Fable",
      channelAssistantName: "Bot",
    }),
    "@Fable",
  );
  assert.equal(
    resolveTermSessionReturnPathMention({
      channelAssistantName: "Bot",
    }),
    "@Bot",
  );
  assert.equal(resolveTermSessionReturnPathMention({}), "");
});

test("resolveTermSessionReturnPathOrigin prefers hulabuzz://message deep link", () => {
  const origin = resolveTermSessionReturnPathOrigin({
    deepLink: `hulabuzz://message?channel=${CHANNEL}&id=${MESSAGE}&thread=${THREAD}`,
    channelId: "wrong-channel",
    threadId: "wrong-thread",
  });
  assert.deepEqual(origin, {
    channelId: CHANNEL,
    threadId: THREAD,
    messageId: MESSAGE,
  });
});

test("resolveTermSessionReturnPathOrigin uses message id as thread when thread= absent", () => {
  const origin = resolveTermSessionReturnPathOrigin({
    deepLink: `buzz://message?channel=${CHANNEL}&id=${MESSAGE}`,
  });
  assert.deepEqual(origin, {
    channelId: CHANNEL,
    threadId: MESSAGE,
    messageId: MESSAGE,
  });
});

test("resolveTermSessionReturnPathOrigin falls back to explicit ids", () => {
  const origin = resolveTermSessionReturnPathOrigin({
    channelId: CHANNEL,
    threadId: THREAD,
  });
  assert.deepEqual(origin, {
    channelId: CHANNEL,
    threadId: THREAD,
    messageId: null,
  });
});

test("resolveTermSessionReturnPathOrigin returns null without usable ids", () => {
  assert.equal(resolveTermSessionReturnPathOrigin({}), null);
  assert.equal(
    resolveTermSessionReturnPathOrigin({ deepLink: "https://example.com" }),
    null,
  );
});

test("buildTermSessionReturnPathSection fills origin + mention + triggers", () => {
  const section = buildTermSessionReturnPathSection({
    originChannelId: CHANNEL,
    originThreadId: THREAD,
    mention: "Fable",
  });
  assert.match(section, /^## Return path \(Buzz\)/);
  assert.match(section, new RegExp(`origin channelId: ${CHANNEL}`));
  assert.match(section, new RegExp(`origin threadId:  ${THREAD}`));
  assert.match(section, /mention to use:   @Fable/);
  assert.match(section, /report back/);
  assert.match(section, /hand back/);
  assert.match(section, /I'm done/);
  assert.match(section, /buzz_draft_message/);
  assert.match(section, /Content = "@Fable <text JM asked for>"/);
  assert.match(section, /Draft only\. JM clicks Send/);
  assert.match(section, /Never draft to any other channel/);
  assert.match(section, /Never auto-draft progress/);
  assert.doesNotMatch(section, /summarized\/source/);
});

test("buildTermSessionReturnPathSection labels summarized when ≠ origin", () => {
  const section = buildTermSessionReturnPathSection({
    originChannelId: CHANNEL,
    originThreadId: THREAD,
    mention: "@ClaimMiner",
    summarizedChannelId: CHANNEL,
    summarizedThreadId: OTHER_THREAD,
  });
  assert.match(section, new RegExp(`origin threadId:  ${THREAD}`));
  assert.match(
    section,
    new RegExp(`summarized/source threadId:  ${OTHER_THREAD}`),
  );
  assert.match(section, /mention to use:   @ClaimMiner/);
});
