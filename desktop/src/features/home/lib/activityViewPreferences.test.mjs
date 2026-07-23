import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_ACTIVITY_CUSTOM_VIEW,
  activityViewStorageKey,
  parseActivityViewPreferences,
} from "./activityViewPreferences.ts";

test("activity view preferences keep a valid saved default and custom mix", () => {
  assert.deepEqual(
    parseActivityViewPreferences({
      version: 1,
      defaultView: "custom",
      custom: {
        dms: true,
        mentions: false,
        agentReplies: false,
      },
    }),
    {
      version: 1,
      defaultView: "custom",
      custom: {
        ...DEFAULT_ACTIVITY_CUSTOM_VIEW,
        mentions: false,
        agentReplies: false,
      },
    },
  );
});

test("activity view preferences reject unknown versions and default invalid views to All", () => {
  assert.equal(
    parseActivityViewPreferences({ version: 2, defaultView: "custom" }),
    null,
  );
  assert.deepEqual(
    parseActivityViewPreferences({
      version: 1,
      defaultView: "surprise",
      custom: null,
    }),
    {
      version: 1,
      defaultView: "all",
      custom: { ...DEFAULT_ACTIVITY_CUSTOM_VIEW },
    },
  );
});

test("activity view preferences are scoped by identity and normalized relay", () => {
  assert.equal(
    activityViewStorageKey("alice", "WSS://EXAMPLE.COM/"),
    activityViewStorageKey("alice", "wss://example.com"),
  );
  assert.notEqual(
    activityViewStorageKey("alice", "wss://one.example"),
    activityViewStorageKey("alice", "wss://two.example"),
  );
  assert.notEqual(
    activityViewStorageKey("alice", "wss://one.example"),
    activityViewStorageKey("bob", "wss://one.example"),
  );
});
