import assert from "node:assert/strict";
import test from "node:test";

import {
  addWelcomeUnreadSuppressedChannelId,
  initialWelcomeUnreadSuppressedChannelIds,
  isWelcomeUnreadSuppressed,
  removeWelcomeUnreadSuppressedChannelId,
} from "./useWelcomeInitialUnreadSuppression.ts";

test("initialWelcomeUnreadSuppressedChannelIds includes the pending active channel", () => {
  const suppressedChannelIds = initialWelcomeUnreadSuppressedChannelIds(
    "welcome",
    (channelId) => channelId === "welcome",
  );

  assert.equal(
    isWelcomeUnreadSuppressed(suppressedChannelIds, "welcome"),
    true,
  );
  assert.equal(
    isWelcomeUnreadSuppressed(suppressedChannelIds, "general"),
    false,
  );
});

test("initial welcome suppression survives after storage is consumed", () => {
  let hasPendingSuppression = true;
  const suppressedChannelIds = initialWelcomeUnreadSuppressedChannelIds(
    "welcome",
    () => hasPendingSuppression,
  );

  hasPendingSuppression = false;

  assert.equal(
    isWelcomeUnreadSuppressed(suppressedChannelIds, "welcome"),
    true,
  );
});

test("welcome unread suppression add and remove are channel scoped", () => {
  const withWelcome = addWelcomeUnreadSuppressedChannelId(new Set(), "welcome");
  const withBoth = addWelcomeUnreadSuppressedChannelId(withWelcome, "design");
  const withoutWelcome = removeWelcomeUnreadSuppressedChannelId(
    withBoth,
    "welcome",
  );

  assert.equal(isWelcomeUnreadSuppressed(withoutWelcome, "welcome"), false);
  assert.equal(isWelcomeUnreadSuppressed(withoutWelcome, "design"), true);
});
