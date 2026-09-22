import assert from "node:assert/strict";
import test from "node:test";

import { initializeI18n } from "@/i18n/index.ts";
import { channelTooltipFooter } from "./ChannelDeepLink.tsx";

// `channelTooltipFooter` resolves its labels through `i18n.t`, which returns
// `undefined` until the singleton boots (`main.tsx` does that in the app).
// Node's own `navigator.languages` reports the host system locale — which may
// be zh-CN — and every assertion below is the English contract.
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
initializeI18n();

const channel = {
  id: "channel-id",
  name: "history",
  channelType: "forum",
  visibility: "private",
  description: "",
  topic: null,
  purpose: null,
  memberCount: 0,
  memberPubkeys: [],
  lastMessageAt: null,
  archivedAt: "2026-08-17T00:00:00Z",
  participants: [],
  participantPubkeys: [],
  isMember: true,
  ttlSeconds: null,
  ttlDeadline: null,
};

test("channelTooltipFooter adds archived status without changing existing metadata", () => {
  assert.equal(
    channelTooltipFooter(channel),
    "Private channel · Forum · Archived",
  );
  assert.equal(
    channelTooltipFooter({ ...channel, archivedAt: null }),
    "Private channel · Forum",
  );
});

test("channelTooltipFooter uses just now for activity within a minute", () => {
  assert.equal(
    channelTooltipFooter({
      ...channel,
      archivedAt: null,
      lastMessageAt: new Date().toISOString(),
    }),
    "Private channel · Forum · Active just now",
  );
});
