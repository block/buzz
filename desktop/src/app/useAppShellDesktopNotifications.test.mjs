import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveThreadReplySenderName,
  threadReplyNotificationTitle,
} from "./useAppShellDesktopNotifications.ts";

const PUBKEY = "a".repeat(64);

test("thread reply title uses resolved sender name with channel", () => {
  const senderName = resolveThreadReplySenderName(PUBKEY, {
    displayName: "Maya",
    nip05Handle: null,
  });

  assert.equal(senderName, "Maya");
  assert.equal(
    threadReplyNotificationTitle(senderName, "#design"),
    "Maya replied in #design",
  );
});

test("thread reply title falls back to Reply when profile is not cached", () => {
  const senderName = resolveThreadReplySenderName(PUBKEY, undefined);

  assert.equal(senderName, undefined);
  assert.equal(
    threadReplyNotificationTitle(senderName, "#design"),
    "Reply in #design",
  );
});

test("empty display name never leaks a truncated pubkey into the title", () => {
  const senderName = resolveThreadReplySenderName(PUBKEY, {
    displayName: "   ",
    nip05Handle: null,
  });

  assert.equal(senderName, undefined);
  assert.equal(threadReplyNotificationTitle(senderName, null), "Reply");
});

test("nip05 handle is used when display name is missing", () => {
  const senderName = resolveThreadReplySenderName(PUBKEY, {
    displayName: null,
    nip05Handle: "maya@example.com",
  });

  assert.equal(senderName, "maya@example.com");
  assert.equal(
    threadReplyNotificationTitle(senderName, "#design"),
    "maya@example.com replied in #design",
  );
});

test("resolved sender title omits channel when channel is unresolved", () => {
  assert.equal(threadReplyNotificationTitle("Maya", null), "Maya replied");
});
