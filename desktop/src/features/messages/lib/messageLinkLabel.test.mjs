import assert from "node:assert/strict";
import test from "node:test";

import {
  getMessageLinkChannelLabel,
  getMessageLinkLabel,
  getMessageLinkPrefix,
} from "./messageLinkLabel.ts";

test("ordinary message links expose an Inbox-style prefix and channel label", () => {
  assert.equal(getMessageLinkPrefix(), "Thread in");
  assert.equal(getMessageLinkChannelLabel("general"), "#general");
});

test("ordinary message links name their target thread", () => {
  assert.equal(
    getMessageLinkLabel({ channelName: "general" }),
    "Thread in #general",
  );
});

test("ordinary message links include a provided root excerpt", () => {
  assert.equal(
    getMessageLinkLabel({
      channelName: "general",
      threadExcerpt: "Release notes",
    }),
    "Thread in #general — Release notes",
  );
});

test("sent-from-thread links use the excerpt as their visible link", () => {
  assert.equal(
    getMessageLinkLabel({
      channelName: "general",
      threadExcerpt: "Release notes",
      variant: "sent-from-thread",
    }),
    "Release notes",
  );
});

test("the prefix resolves from the catalog in the active language", async () => {
  await i18n.changeLanguage("zh-Hans");
  try {
    assert.equal(getMessageLinkPrefix(), "来自讨论串");
    assert.equal(
      getMessageLinkLabel({ channelName: "general" }),
      "来自讨论串 #general",
    );
  } finally {
    await i18n.changeLanguage("en");
  }
});

// The label is catalog copy now, so the module has to be booted before any test
// body runs; the assertions above keep their exact English expectations.
import { i18n, initializeI18n } from "@/i18n";

Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
initializeI18n();
