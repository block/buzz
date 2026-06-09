import assert from "node:assert/strict";
import test from "node:test";

import {
  buildMessageLink,
  isMessageLink,
  parseMessageLink,
} from "./messageLink.ts";

const CHANNEL = "f570339f-8f8a-4e08-a779-8d954aa44109";
const MESSAGE =
  "b04819ffc1f7c8ffb49c6d30b5899f470198264680d02e78894a658e30a9059f";
const THREAD =
  "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

test("buildMessageLink → parseMessageLink round-trips without thread", () => {
  const url = buildMessageLink({ channelId: CHANNEL, messageId: MESSAGE });
  assert.equal(url, `sprout://message?channel=${CHANNEL}&id=${MESSAGE}`);

  const parsed = parseMessageLink(url);
  assert.equal(parsed.ok, true);
  assert.deepEqual(parsed.ok && parsed.value, {
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: null,
  });
});

test("buildMessageLink → parseMessageLink round-trips with thread", () => {
  const url = buildMessageLink({
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: THREAD,
  });
  const parsed = parseMessageLink(url);
  assert.equal(parsed.ok, true);
  assert.deepEqual(parsed.ok && parsed.value, {
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: THREAD,
  });
});

test("buildMessageLink treats null/empty thread as absent", () => {
  const a = buildMessageLink({
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: null,
  });
  const b = buildMessageLink({
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: "",
  });
  assert.equal(a, `sprout://message?channel=${CHANNEL}&id=${MESSAGE}`);
  assert.equal(b, `sprout://message?channel=${CHANNEL}&id=${MESSAGE}`);
});

test("buildMessageLink rejects missing required params", () => {
  assert.throws(() => buildMessageLink({ channelId: "", messageId: MESSAGE }));
  assert.throws(() => buildMessageLink({ channelId: CHANNEL, messageId: "" }));
});

test("parseMessageLink rejects non-sprout schemes", () => {
  const r = parseMessageLink(
    `https://example.com/?channel=${CHANNEL}&id=${MESSAGE}`,
  );
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "wrong-scheme");
});

test("parseMessageLink rejects sprout:// with wrong host", () => {
  const r = parseMessageLink(`sprout://connect?relay=wss://example.com`);
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "wrong-host");
});

test("parseMessageLink rejects missing channel", () => {
  const r = parseMessageLink(`sprout://message?id=${MESSAGE}`);
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "missing-channel");
});

test("parseMessageLink rejects missing id", () => {
  const r = parseMessageLink(`sprout://message?channel=${CHANNEL}`);
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "missing-id");
});

test("parseMessageLink rejects malformed URL strings", () => {
  const r = parseMessageLink("not a url");
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "invalid-url");
});

test("isMessageLink matches only sprout://message", () => {
  assert.equal(
    isMessageLink(`sprout://message?channel=${CHANNEL}&id=${MESSAGE}`),
    true,
  );
  assert.equal(isMessageLink("sprout://connect?relay=wss://x"), false);
  assert.equal(isMessageLink("https://example.com"), false);
  assert.equal(isMessageLink(undefined), false);
  assert.equal(isMessageLink(""), false);
});
