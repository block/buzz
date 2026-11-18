import assert from "node:assert/strict";
import test from "node:test";

import { formatMessageNotification } from "./notificationFormat.ts";
import { senderNameFromSummary } from "./senderName.ts";

test("DM title is the sender name when resolved", () => {
  const { title, body } = formatMessageNotification({
    source: "dm",
    senderName: "Taylor",
    channelName: "taylor-wes",
    content: "hey there",
  });

  assert.equal(title, "Taylor");
  assert.equal(body, "hey there");
});

test("DM title falls back to the channel name, then generic copy", () => {
  assert.equal(
    formatMessageNotification({
      source: "dm",
      senderName: null,
      channelName: "taylor-wes",
      content: "hi",
    }).title,
    "taylor-wes",
  );
  assert.equal(
    formatMessageNotification({
      source: "dm",
      senderName: "  ",
      channelName: "",
      content: "hi",
    }).title,
    "Direct message",
  );
});

test("DM body falls back when the message is blank", () => {
  assert.equal(
    formatMessageNotification({
      source: "dm",
      senderName: "Taylor",
      channelName: null,
      content: "   ",
    }).body,
    "New message",
  );
});

test("thread reply leads with the sender when resolved", () => {
  assert.equal(
    formatMessageNotification({
      source: "thread_reply",
      senderName: "Taylor",
      channelName: "ship-room",
      content: "done!",
    }).title,
    "Taylor replied in #ship-room",
  );
});

test("thread reply preserves legacy copy when the sender is unknown", () => {
  assert.equal(
    formatMessageNotification({
      source: "thread_reply",
      senderName: null,
      channelName: "ship-room",
      content: "done!",
    }).title,
    "Reply in #ship-room",
  );
  assert.deepEqual(
    formatMessageNotification({
      source: "thread_reply",
      senderName: null,
      channelName: null,
      content: "",
    }),
    { title: "Reply", body: "New reply" },
  );
});

test("mention titles match the home-feed conventions", () => {
  assert.equal(
    formatMessageNotification({
      source: "mention",
      senderName: "Taylor",
      channelName: "ship-room",
      content: "@wes look",
    }).title,
    "Taylor mentioned you in #ship-room",
  );
  assert.equal(
    formatMessageNotification({
      source: "mention",
      senderName: null,
      channelName: "ship-room",
      content: "@wes look",
    }).title,
    "@Mention in #ship-room",
  );
});

test("approval and needs-action titles match the home-feed conventions", () => {
  assert.deepEqual(
    formatMessageNotification({
      source: "approval",
      senderName: "Taylor",
      channelName: "ops",
      content: "",
    }),
    {
      title: "Taylor requested approval in #ops",
      body: "A workflow is waiting for your approval.",
    },
  );
  assert.deepEqual(
    formatMessageNotification({
      source: "needs_action",
      senderName: null,
      channelName: null,
      content: "",
    }),
    {
      title: "Needs Action",
      body: "Something in Buzz needs your attention.",
    },
  );
});

test("senderNameFromSummary prefers displayName, then NIP-05, never a pubkey", () => {
  assert.equal(
    senderNameFromSummary({
      displayName: "Taylor",
      avatarUrl: null,
      nip05Handle: "taylor@buzz.example",
      ownerPubkey: null,
    }),
    "Taylor",
  );
  assert.equal(
    senderNameFromSummary({
      displayName: "  ",
      avatarUrl: null,
      nip05Handle: "taylor@buzz.example",
      ownerPubkey: null,
    }),
    "taylor@buzz.example",
  );
  assert.equal(
    senderNameFromSummary({
      displayName: null,
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    }),
    null,
  );
  assert.equal(senderNameFromSummary(null), null);
  assert.equal(senderNameFromSummary(undefined), null);
});
